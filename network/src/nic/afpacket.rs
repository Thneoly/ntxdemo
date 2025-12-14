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
    last_pkttype: Option<u8>,
}

/// AF_PACKET cooked capture backend.
///
/// Uses `SOCK_DGRAM` which yields a "Linux cooked" header (SLL/SLL2) instead of a
/// full Ethernet header. This can be more reliable on some virtual devices.
///
/// To keep the rest of the stack unchanged, `recv_nonblocking()` prepends a
/// synthetic Ethernet header (dst=broadcast, src=iface_mac|0, ethertype derived from
/// sockaddr_ll::sll_protocol) so `PacketContext::decode()` can continue parsing Ethernet.
///
/// TX contract:
/// - `send()` accepts either a full Ethernet+IPv4 frame or a raw IPv4 packet.
///   For Ethernet+IPv4, it strips Ethernet and transmits at L3.
/// - Non-IPv4 TX is currently unsupported.
#[allow(dead_code)]
pub struct AfPacketDgramNic {
    fd: RawFd,
    ifindex: i32,
    ifname: String,
    ifmac: Option<[u8; 6]>,
    last_pkttype: Option<u8>,
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
        // Be permissive about packet direction/classification; veth can be surprising.
        sll.sll_pkttype = libc::PACKET_OTHERHOST as u8;

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

        // On some setups (notably veth + netns tests), the frames we care about may
        // appear as "outgoing" on the host side. Make sure the socket does NOT
        // ignore outgoing frames.
        //
        // PACKET_IGNORE_OUTGOING defaults vary by kernel/config; set explicitly.
        let zero: libc::c_int = 0;
        let _ = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_PACKET,
                libc::PACKET_IGNORE_OUTGOING,
                &zero as *const _ as *const libc::c_void,
                mem::size_of_val(&zero) as u32,
            )
        };

        // Enable PACKET_AUXDATA so recvmsg() can provide per-packet metadata
        // (e.g. vlan info, checksum status). We use it primarily as a canary
        // that recvmsg control parsing works.
        let one: libc::c_int = 1;
        let _ = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_PACKET,
                libc::PACKET_AUXDATA,
                &one as *const _ as *const libc::c_void,
                mem::size_of_val(&one) as u32,
            )
        };

        let ifmac = get_iface_mac(ifname).ok();

        Ok(Self {
            fd,
            ifindex,
            ifname: ifname.to_string(),
            ifmac,
            last_pkttype: None,
        })
    }

    /// Last observed packet type (sll_pkttype) from sockaddr_ll.
    /// Useful for debugging veth setups where traffic can appear as PACKET_OUTGOING.
    #[allow(dead_code)]
    pub fn last_pkttype(&self) -> Option<u8> {
        self.last_pkttype
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
    pub fn recv_nonblocking(&mut self, buf: &mut [u8]) -> anyhow::Result<Option<usize>> {
        // Use recvmsg so we can capture sockaddr_ll.sll_pkttype.
        let mut iov = libc::iovec {
            iov_base: buf.as_mut_ptr() as *mut libc::c_void,
            iov_len: buf.len(),
        };

        let mut addr: libc::sockaddr_ll = unsafe { mem::zeroed() };
        let addrlen: libc::socklen_t = mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t;

        // Control buffer for PACKET_AUXDATA; keep generous.
        let mut cmsg_buf = [0u8; 256];

        let mut msg: libc::msghdr = unsafe { mem::zeroed() };
        msg.msg_name = (&mut addr as *mut libc::sockaddr_ll) as *mut libc::c_void;
        msg.msg_namelen = addrlen;
        msg.msg_iov = &mut iov as *mut libc::iovec;
        msg.msg_iovlen = 1;
        msg.msg_control = cmsg_buf.as_mut_ptr() as *mut libc::c_void;
        msg.msg_controllen = cmsg_buf.len();

        let n =
            unsafe { libc::recvmsg(self.fd, &mut msg as *mut libc::msghdr, libc::MSG_DONTWAIT) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(e).context("recvmsg(AF_PACKET, MSG_DONTWAIT) failed");
        }

        // Record pkttype for debugging.
        self.last_pkttype = Some(addr.sll_pkttype);
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

impl AfPacketDgramNic {
    #[allow(dead_code)]
    pub fn open(ifname: &str) -> anyhow::Result<Self> {
        let fd = unsafe {
            libc::socket(
                libc::AF_PACKET,
                libc::SOCK_DGRAM,
                (libc::ETH_P_ALL as u16).to_be() as i32,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error())
                .context("socket(AF_PACKET, SOCK_DGRAM) failed");
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
            return Err(e).context("bind(AF_PACKET, SOCK_DGRAM) failed");
        }

        let ifmac = get_iface_mac(ifname).ok();

        Ok(Self {
            fd,
            ifindex,
            ifname: ifname.to_string(),
            ifmac,
            last_pkttype: None,
        })
    }

    #[allow(dead_code)]
    pub fn recv_nonblocking(&mut self, out: &mut [u8]) -> anyhow::Result<Option<usize>> {
        // Need room for a synthetic Ethernet header.
        if out.len() < crate::EthernetHeader::LEN {
            anyhow::bail!("buffer too small for cooked recv: {}", out.len());
        }

        // Receive cooked payload right after synthetic Ethernet header.
        let payload_buf = &mut out[crate::EthernetHeader::LEN..];
        let mut iov = libc::iovec {
            iov_base: payload_buf.as_mut_ptr() as *mut libc::c_void,
            iov_len: payload_buf.len(),
        };

        let mut addr: libc::sockaddr_ll = unsafe { mem::zeroed() };
        let addrlen: libc::socklen_t = mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t;

        let mut msg: libc::msghdr = unsafe { mem::zeroed() };
        msg.msg_name = (&mut addr as *mut libc::sockaddr_ll) as *mut libc::c_void;
        msg.msg_namelen = addrlen;
        msg.msg_iov = &mut iov as *mut libc::iovec;
        msg.msg_iovlen = 1;

        let n =
            unsafe { libc::recvmsg(self.fd, &mut msg as *mut libc::msghdr, libc::MSG_DONTWAIT) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(e).context("recvmsg(AF_PACKET, SOCK_DGRAM, MSG_DONTWAIT) failed");
        }

        self.last_pkttype = Some(addr.sll_pkttype);

        // Build synthetic ethernet header so the rest of the parser keeps working.
        // We don't know L2 src/dst from cooked mode; that's OK for UDP reply builder
        // because it uses decoded metadata for dst/src MAC. For now use BROADCAST dst
        // and our iface mac as src if available.
        let dst = crate::MacAddr::BROADCAST;
        let src = crate::MacAddr(self.ifmac.unwrap_or([0u8; 6]));
        // In cooked mode, sockaddr_ll::sll_protocol is the L3 protocol in network byte order,
        // but its *values* are Linux ETH_P_* constants (e.g. ETH_P_IP=0x0800).
        // Convert to host order so downstream sees standard ethertypes.
        let ethertype = u16::from_be(addr.sll_protocol);
        let eth = crate::EthernetHeader {
            dst,
            src,
            ethertype,
        };
        eth.write(&mut out[..crate::EthernetHeader::LEN])?;

        Ok(Some(crate::EthernetHeader::LEN + n as usize))
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

    fn last_pkttype(&self) -> Option<u8> {
        self.last_pkttype
    }
}

impl Nic for AfPacketDgramNic {
    fn ifindex(&self) -> i32 {
        self.ifindex
    }

    fn ifname(&self) -> &str {
        &self.ifname
    }

    fn iface_mac(&self) -> Option<[u8; 6]> {
        self.ifmac
    }

    fn send(&self, frame: &[u8]) -> anyhow::Result<usize> {
        // AF_PACKET/SOCK_DGRAM is L3-oriented.
        // Accept either:
        // - Ethernet+IPv4: [Ethernet][IPv4...], we'll strip Ethernet.
        // - Raw IPv4: [IPv4...], we'll send as-is.
        if frame.is_empty() {
            anyhow::bail!("frame too small: 0");
        }

        let (l3, proto): (&[u8], u16) = if frame.len() >= crate::EthernetHeader::LEN {
            let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
            if ethertype == crate::ETH_TYPE_IPV4 {
                (&frame[crate::EthernetHeader::LEN..], libc::ETH_P_IP as u16)
            } else {
                // Not Ethernet+IPv4; maybe it's raw IPv4.
                let ver = frame[0] >> 4;
                if ver == 4 {
                    (frame, libc::ETH_P_IP as u16)
                } else {
                    anyhow::bail!(
                        "AfPacketDgramNic: unsupported send buffer (ethertype=0x{ethertype:04x}, ver={ver})"
                    );
                }
            }
        } else {
            // Too short to be Ethernet; must be raw IPv4.
            let ver = frame[0] >> 4;
            if ver == 4 {
                (frame, libc::ETH_P_IP as u16)
            } else {
                anyhow::bail!(
                    "AfPacketDgramNic: buffer too small / not IPv4 (len={})",
                    frame.len()
                );
            }
        };

        let mut sll: libc::sockaddr_ll = unsafe { mem::zeroed() };
        sll.sll_family = libc::AF_PACKET as u16;
        sll.sll_protocol = proto.to_be();
        sll.sll_ifindex = self.ifindex;

        let n = unsafe {
            libc::sendto(
                self.fd,
                l3.as_ptr() as *const libc::c_void,
                l3.len(),
                0,
                &sll as *const libc::sockaddr_ll as *const libc::sockaddr,
                mem::size_of::<libc::sockaddr_ll>() as u32,
            )
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error())
                .context("sendto(AF_PACKET, SOCK_DGRAM) failed");
        }
        Ok(n as usize)
    }

    fn recv(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        // Blocking form: poll then nonblocking.
        loop {
            if let Some(n) = self.recv_nonblocking(buf)? {
                return Ok(n);
            }
            self.poll_readable(None)?;
        }
    }

    fn recv_nonblocking(&mut self, buf: &mut [u8]) -> anyhow::Result<Option<usize>> {
        AfPacketDgramNic::recv_nonblocking(self, buf)
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
            return Err(std::io::Error::last_os_error())
                .context("poll(AF_PACKET, SOCK_DGRAM) failed");
        }
        Ok(rc > 0 && (pfd.revents & libc::POLLIN) != 0)
    }

    fn last_pkttype(&self) -> Option<u8> {
        self.last_pkttype
    }
}

impl Drop for AfPacketDgramNic {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
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
