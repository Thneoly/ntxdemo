use anyhow::Context;
use std::ffi::CString;
use std::mem;
use std::os::fd::RawFd;
use std::ptr;
use std::time::Duration;

use super::Nic;
use libc::tpacket_versions::TPACKET_V3;

/// PACKET_RX_RING / TPACKET_V3 backend.
///
/// Notes / scope:
/// - RX uses `PACKET_RX_RING` with `TPACKET_V3` and a single mmap region.
/// - RX iterates **all packets within a completed block** (TP_STATUS_USER), but preserves
///   the `Nic` trait's one-frame-per-call semantics: each `recv*()` returns **at most one**
///   frame; subsequent calls continue from the same block until it is fully consumed.
/// - TX uses `sendto()` on the same AF_PACKET socket (copy path TX).
/// - This backend is Linux-specific.
///
/// Design goal: keep upper layers unchanged by implementing [`Nic`].
#[allow(dead_code)]
pub struct TpacketV3Nic {
    fd: RawFd,
    ifindex: i32,
    ifname: String,
    ifmac: Option<[u8; 6]>,

    /// mmap()'d ring.
    ring_ptr: *mut u8,
    ring_len: usize,

    /// tpacket_req3 config.
    req: libc::tpacket_req3,

    /// Current block index.
    cur_block: u32,

    /// When we have a completed block (TP_STATUS_USER), this is the next packet
    /// index within that block to return.
    ///
    /// Invariant: `cur_pkt_in_block == 0` means we have not started consuming
    /// the current `cur_block` yet (or we just released it).
    cur_pkt_in_block: u32,
}

unsafe impl Send for TpacketV3Nic {}
unsafe impl Sync for TpacketV3Nic {}

impl TpacketV3Nic {
    /// Open a TPACKET_V3 socket with PACKET_RX_RING.
    ///
    /// `block_size` and `block_nr` control ring size. A reasonable start for veth
    /// tests is 1MiB blocks and 64 blocks (~64MiB).
    #[allow(dead_code)]
    pub fn open(
        ifname: &str,
        block_size: u32,
        block_nr: u32,
        frame_size: u32,
        retire_blk_tov_ms: u32,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            block_size.is_power_of_two(),
            "block_size must be power-of-two"
        );
        anyhow::ensure!(block_nr > 0, "block_nr must be > 0");
        anyhow::ensure!(frame_size > 0, "frame_size must be > 0");

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

        // PACKET_VERSION = TPACKET_V3
        let ver: libc::c_int = TPACKET_V3 as libc::c_int;
        let rc = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_PACKET,
                libc::PACKET_VERSION,
                &ver as *const _ as *const libc::c_void,
                mem::size_of_val(&ver) as u32,
            )
        };
        if rc != 0 {
            let e = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e).context("setsockopt(PACKET_VERSION=TPACKET_V3) failed");
        }

        let c_ifname = CString::new(ifname)?;
        let ifindex = unsafe { libc::if_nametoindex(c_ifname.as_ptr()) } as i32;
        if ifindex == 0 {
            let e = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e).context("if_nametoindex failed");
        }

        // Configure RX ring.
        // See `man 7 packet` and `struct tpacket_req3`.
        let mut req: libc::tpacket_req3 = unsafe { mem::zeroed() };
        req.tp_block_size = block_size;
        req.tp_block_nr = block_nr;
        req.tp_frame_size = frame_size;
        req.tp_frame_nr = (block_size as u64 * block_nr as u64 / frame_size as u64) as u32;
        req.tp_retire_blk_tov = retire_blk_tov_ms;
        req.tp_sizeof_priv = 0;
        req.tp_feature_req_word = 0;

        let rc = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_PACKET,
                libc::PACKET_RX_RING,
                &req as *const _ as *const libc::c_void,
                mem::size_of_val(&req) as u32,
            )
        };
        if rc != 0 {
            let e = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e).context("setsockopt(PACKET_RX_RING) failed");
        }

        // Ensure the ring delivers full L2 frames (include Ethernet header).
        // Some kernels default to delivering from L3 depending on packet type/offsets.
        // Setting PACKET_HDRLEN to ETH_HLEN is a harmless hint and improves portability.
        let hdrlen: libc::c_int = 14;
        let _ = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_PACKET,
                libc::PACKET_HDRLEN,
                &hdrlen as *const _ as *const libc::c_void,
                mem::size_of_val(&hdrlen) as u32,
            )
        };

        let ring_len = (req.tp_block_size as usize) * (req.tp_block_nr as usize);
        let ring_ptr = unsafe {
            libc::mmap(
                ptr::null_mut(),
                ring_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if ring_ptr == libc::MAP_FAILED {
            let e = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e).context("mmap(PACKET_RX_RING) failed");
        }

        // Bind to interface.
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
            unsafe {
                libc::munmap(ring_ptr, ring_len);
                libc::close(fd);
            }
            return Err(e).context("bind(AF_PACKET) failed");
        }

        // We want to see incoming frames to the interface. Some environments only deliver
        // packets to non-promiscuous AF_PACKET sockets if they are destined to the host.
        // In our veth tests, we expect to receive frames addressed to our NIC MAC, but
        // enabling promiscuous mode removes any ambiguity (and matches typical sniffer behavior).
        let mreq = libc::packet_mreq {
            mr_ifindex: ifindex,
            mr_type: libc::PACKET_MR_PROMISC as u16,
            mr_alen: 0,
            mr_address: [0u8; 8],
        };
        let _ = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_PACKET,
                libc::PACKET_ADD_MEMBERSHIP,
                &mreq as *const _ as *const libc::c_void,
                mem::size_of_val(&mreq) as u32,
            )
        };

        let ifmac = super::afpacket::get_iface_mac(ifname).ok();

        Ok(Self {
            fd,
            ifindex,
            ifname: ifname.to_string(),
            ifmac,
            ring_ptr: ring_ptr as *mut u8,
            ring_len,
            req,
            cur_block: 0,
            cur_pkt_in_block: 0,
        })
    }

    #[inline]
    fn block_ptr(&self, block_idx: u32) -> *mut libc::tpacket_block_desc {
        unsafe {
            self.ring_ptr
                .add(block_idx as usize * self.req.tp_block_size as usize)
                as *mut libc::tpacket_block_desc
        }
    }

    /// Try to receive one frame from the ring.
    ///
    /// Returns `Ok(None)` if no completed block is available right now.
    fn recv_one(&mut self, buf: &mut [u8]) -> anyhow::Result<Option<usize>> {
        // Semantics: return ONE frame per call, but keep cursor state so that
        // multiple frames within the same completed block can be returned across
        // successive calls.
        let bdesc = self.block_ptr(self.cur_block);
        let status = unsafe { (*bdesc).hdr.bh1.block_status };
        if (status & (libc::TP_STATUS_USER as u32)) == 0 {
            return Ok(None);
        }

        let num_pkts = unsafe { (*bdesc).hdr.bh1.num_pkts };
        if num_pkts == 0 {
            // Release empty block.
            unsafe {
                (*bdesc).hdr.bh1.block_status = libc::TP_STATUS_KERNEL as u32;
            }
            self.cur_block = (self.cur_block + 1) % self.req.tp_block_nr;
            self.cur_pkt_in_block = 0;
            return Ok(None);
        }

        // If we've already consumed everything in this block, release it.
        if self.cur_pkt_in_block >= num_pkts {
            unsafe {
                (*bdesc).hdr.bh1.block_status = libc::TP_STATUS_KERNEL as u32;
            }
            self.cur_block = (self.cur_block + 1) % self.req.tp_block_nr;
            self.cur_pkt_in_block = 0;
            return Ok(None);
        }

        // The first packet starts at `offset_to_first_pkt`. Each `tpacket3_hdr`
        // contains `tp_next_offset` to jump to the next packet.
        let mut off = unsafe { (*bdesc).hdr.bh1.offset_to_first_pkt } as usize;
        for _ in 0..self.cur_pkt_in_block {
            let tph = unsafe { (bdesc as *mut u8).add(off) as *mut libc::tpacket3_hdr };
            let next = unsafe { (*tph).tp_next_offset } as usize;
            if next == 0 {
                // Defensive: malformed block; release it.
                unsafe {
                    (*bdesc).hdr.bh1.block_status = libc::TP_STATUS_KERNEL as u32;
                }
                self.cur_block = (self.cur_block + 1) % self.req.tp_block_nr;
                self.cur_pkt_in_block = 0;
                return Ok(None);
            }
            off += next;
        }

        let tph = unsafe { (bdesc as *mut u8).add(off) as *mut libc::tpacket3_hdr };
        let snaplen = unsafe { (*tph).tp_snaplen } as usize;
        // NOTE: For TPACKET_V3, `tp_mac` points to the L2 header (Ethernet).
        // `tp_net` points to the L3 header (typically IPv4).
        // Our upper layers expect a full Ethernet frame, so we must copy from `tp_mac`.
        // Some kernels/drivers may populate `tp_mac` as 0; fall back to `tp_net - 14`
        // in that case.
        let mac_off = {
            let m = unsafe { (*tph).tp_mac } as usize;
            if m != 0 {
                m
            } else {
                let net = unsafe { (*tph).tp_net } as usize;
                net.saturating_sub(14)
            }
        };
        let pkt_ptr = unsafe { (tph as *mut u8).add(mac_off) };

        let n = snaplen.min(buf.len());

        // IMPORTANT: `tp_snaplen` is the captured length; in practice it is most reliable
        // when interpreted as the L2 byte length in our environment (veth + ETH_P_ALL).
        // Earlier versions of this code assumed snaplen started at `tp_net` (L3), but that
        // results in truncation and decode failures (udp=0) on veth.
        let net_off = unsafe { (*tph).tp_net } as usize;
        let _ = net_off; // keep for future debug/use

        let l2_len = n;

        unsafe {
            ptr::copy_nonoverlapping(pkt_ptr as *const u8, buf.as_mut_ptr(), l2_len);
        }

        self.cur_pkt_in_block += 1;

        // If this was the last packet in the block, release it back to kernel.
        if self.cur_pkt_in_block >= num_pkts {
            unsafe {
                (*bdesc).hdr.bh1.block_status = libc::TP_STATUS_KERNEL as u32;
            }
            self.cur_block = (self.cur_block + 1) % self.req.tp_block_nr;
            self.cur_pkt_in_block = 0;
        }

        Ok(Some(l2_len))
    }
}

impl Nic for TpacketV3Nic {
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
        // Same as AF_PACKET sendto(). Destination MAC is in the frame.
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

    fn recv(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        loop {
            if let Some(n) = self.recv_one(buf)? {
                return Ok(n);
            }
            // Block until readable.
            self.poll_readable(None)?;
        }
    }

    fn recv_nonblocking(&mut self, buf: &mut [u8]) -> anyhow::Result<Option<usize>> {
        self.recv_one(buf)
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
            return Err(std::io::Error::last_os_error()).context("poll(TPACKETv3) failed");
        }
        Ok(rc > 0 && (pfd.revents & libc::POLLIN) != 0)
    }
}

impl Drop for TpacketV3Nic {
    fn drop(&mut self) {
        unsafe {
            // best-effort disable ring
            let req0: libc::tpacket_req3 = mem::zeroed();
            let _ = libc::setsockopt(
                self.fd,
                libc::SOL_PACKET,
                libc::PACKET_RX_RING,
                &req0 as *const _ as *const libc::c_void,
                mem::size_of_val(&req0) as u32,
            );
            if !self.ring_ptr.is_null() {
                let _ = libc::munmap(self.ring_ptr as *mut libc::c_void, self.ring_len);
            }
            libc::close(self.fd);
        }
    }
}
