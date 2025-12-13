use anyhow::Context;
use std::ffi::CString;
use std::mem;
use std::os::fd::RawFd;
use std::time::Duration;

use super::Nic;

/// AF_PACKET (PF_PACKET) raw socket NIC backend.
///
/// This is a **copy** path, but it works on most NICs and is great for building a
/// userspace protocol stack without relying on the kernel IP stack.
#[allow(dead_code)]
pub struct AfPacketNic {
    fd: RawFd,
    ifindex: i32,
    ifname: String,
    ifmac: Option<[u8; 6]>,
}

impl AfPacketNic {
    #[allow(dead_code)]
    pub fn open(ifname: &str) -> anyhow::Result<Self> {
        let fd = unsafe {
            libc::socket(
                libc::AF_PACKET,
                libc::SOCK_RAW,
                (libc::ETH_P_ALL as u16).to_be() as i32,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error()).context("socket(AF_PACKET) failed");
        }

        let c_ifname = CString::new(ifname)?;
        let ifindex = unsafe { libc::if_nametoindex(c_ifname.as_ptr()) } as i32;
        if ifindex == 0 {
            let e = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e).context("if_nametoindex failed");
        }

        let mut sll: libc::sockaddr_ll = unsafe { mem::zeroed() };
        sll.sll_family = libc::AF_PACKET as u16;
        sll.sll_protocol = (libc::ETH_P_ALL as u16).to_be();
        sll.sll_ifindex = ifindex;

        let rc = unsafe {
            libc::bind(
                fd,
                &sll as *const libc::sockaddr_ll as *const libc::sockaddr,
                mem::size_of::<libc::sockaddr_ll>() as u32,
            )
        };
        if rc != 0 {
            let e = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e).context("bind(AF_PACKET) failed");
        }

        let ifmac = get_iface_mac(ifname).ok();

        Ok(Self {
            fd,
            ifindex,
            ifname: ifname.to_string(),
            ifmac,
        })
    }

    #[allow(dead_code)]
    pub fn ifindex(&self) -> i32 {
        self.ifindex
    }

    #[allow(dead_code)]
    pub fn ifname(&self) -> &str {
        &self.ifname
    }

    /// Returns the interface MAC address if available.
    ///
    /// On Linux this is queried via ioctl(SIOCGIFHWADDR). If the ioctl fails
    /// (e.g. permission / not supported), this returns None.
    #[allow(dead_code)]
    pub fn iface_mac(&self) -> Option<[u8; 6]> {
        self.ifmac
    }

    #[allow(dead_code)]
    pub fn recv(&self, buf: &mut [u8]) -> anyhow::Result<usize> {
        let n = unsafe { libc::recv(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
        if n < 0 {
            return Err(std::io::Error::last_os_error()).context("recv(AF_PACKET) failed");
        }
        Ok(n as usize)
    }

    /// Non-blocking receive.
    ///
    /// Returns:
    /// - Ok(Some(n)) when a frame is received
    /// - Ok(None) when there is no frame available right now (EAGAIN/EWOULDBLOCK)
    /// - Err(e) for other errors
    #[allow(dead_code)]
    pub fn recv_nonblocking(&self, buf: &mut [u8]) -> anyhow::Result<Option<usize>> {
        let n = unsafe {
            libc::recv(
                self.fd,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
                libc::MSG_DONTWAIT,
            )
        };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            // WouldBlock means no packet ready.
            if e.kind() == std::io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(e).context("recv(AF_PACKET, MSG_DONTWAIT) failed");
        }
        Ok(Some(n as usize))
    }

    #[allow(dead_code)]
    pub fn send(&self, frame: &[u8]) -> anyhow::Result<usize> {
        // For AF_PACKET, sendto() needs a sockaddr_ll; destination MAC is taken from the frame.
        let mut sll: libc::sockaddr_ll = unsafe { mem::zeroed() };
        sll.sll_family = libc::AF_PACKET as u16;
        sll.sll_protocol = (libc::ETH_P_ALL as u16).to_be();
        sll.sll_ifindex = self.ifindex;

        let n = unsafe {
            libc::sendto(
                self.fd,
                frame.as_ptr() as *const libc::c_void,
                frame.len(),
                0,
                &sll as *const libc::sockaddr_ll as *const libc::sockaddr,
                mem::size_of::<libc::sockaddr_ll>() as u32,
            )
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error()).context("sendto(AF_PACKET) failed");
        }
        Ok(n as usize)
    }
}

impl Nic for AfPacketNic {
    fn ifindex(&self) -> i32 {
        self.ifindex()
    }

    fn ifname(&self) -> &str {
        self.ifname()
    }

    fn iface_mac(&self) -> Option<[u8; 6]> {
        self.iface_mac()
    }

    fn send(&self, frame: &[u8]) -> anyhow::Result<usize> {
        AfPacketNic::send(self, frame)
    }

    fn recv(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        AfPacketNic::recv(self, buf)
    }

    fn recv_nonblocking(&mut self, buf: &mut [u8]) -> anyhow::Result<Option<usize>> {
        AfPacketNic::recv_nonblocking(self, buf)
    }

    fn poll_readable(&self, timeout: Option<Duration>) -> anyhow::Result<bool> {
        let mut pfd = libc::pollfd {
            fd: self.fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let to_ms: i32 = match timeout {
            None => -1,
            Some(d) => {
                let ms = d.as_millis();
                if ms > i32::MAX as u128 {
                    i32::MAX
                } else {
                    ms as i32
                }
            }
        };

        let rc = unsafe { libc::poll(&mut pfd as *mut libc::pollfd, 1, to_ms) };
        if rc < 0 {
            return Err(std::io::Error::last_os_error()).context("poll(AF_PACKET) failed");
        }
        Ok(rc > 0 && (pfd.revents & libc::POLLIN) != 0)
    }
}

pub(super) fn get_iface_mac(ifname: &str) -> anyhow::Result<[u8; 6]> {
    use std::io;

    // Create a temporary socket for ioctl. Any IPv4 datagram socket works.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error()).context("socket(AF_INET) for ioctl failed");
    }

    // Prepare ifreq.
    let mut ifr: libc::ifreq = unsafe { mem::zeroed() };
    {
        let name_bytes = ifname.as_bytes();
        if name_bytes.len() >= libc::IFNAMSIZ {
            unsafe { libc::close(fd) };
            anyhow::bail!("ifname too long: {}", ifname);
        }
        // Safety: ifr_name is a fixed-size [c_char; IFNAMSIZ].
        for (i, b) in name_bytes.iter().enumerate() {
            ifr.ifr_name[i] = *b as libc::c_char;
        }
        ifr.ifr_name[name_bytes.len()] = 0;
    }

    let rc = unsafe { libc::ioctl(fd, libc::SIOCGIFHWADDR, &mut ifr) };
    let res = if rc < 0 {
        Err(io::Error::last_os_error()).context("ioctl(SIOCGIFHWADDR) failed")
    } else {
        // SAFETY: the kernel fills the ifreq union with a sockaddr.
        // For ARPHRD_ETHER, sa_data[0..6] is the MAC.
        let hw = unsafe { ifr.ifr_ifru.ifru_hwaddr.sa_data };
        Ok([
            hw[0] as u8,
            hw[1] as u8,
            hw[2] as u8,
            hw[3] as u8,
            hw[4] as u8,
            hw[5] as u8,
        ])
    };

    unsafe { libc::close(fd) };
    res
}

impl Drop for AfPacketNic {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}
