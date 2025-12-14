//! Pure-logic TCP client state machine.
//!
//! This module does **not** do any IO. You feed it inbound segments and it returns
//! outbound segments to send.

use anyhow::{Result, bail};

use super::{TcpFlags, TcpHeader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpClientState {
    Closed,
    SynSent,
    Established,
    FinWait1,
    FinWait2,
    TimeWait,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcpSegment {
    pub hdr: TcpHeader,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct TcpClient {
    pub local_port: u16,
    pub remote_port: u16,
    pub state: TcpClientState,

    /// Next sequence number to send.
    snd_nxt: u32,
    /// Oldest unacknowledged sequence number.
    snd_una: u32,
    /// Next expected sequence number from remote.
    rcv_nxt: u32,

    pub wnd: u16,
}

impl TcpClient {
    /// Create a client with an initial sequence number.
    pub fn new(local_port: u16, remote_port: u16, isn: u32) -> Self {
        Self {
            local_port,
            remote_port,
            state: TcpClientState::Closed,
            snd_nxt: isn,
            snd_una: isn,
            rcv_nxt: 0,
            wnd: 65535,
        }
    }

    pub fn connect(&mut self) -> Result<TcpSegment> {
        if self.state != TcpClientState::Closed {
            bail!("connect called in state {:?}", self.state);
        }

        // Send SYN, consumes 1 seq.
        let syn_seq = self.snd_nxt;
        self.snd_nxt = self.snd_nxt.wrapping_add(1);
        self.state = TcpClientState::SynSent;

        Ok(TcpSegment {
            hdr: TcpHeader {
                src_port: self.local_port,
                dst_port: self.remote_port,
                seq: syn_seq,
                ack: 0,
                data_offset_words: 5,
                flags: TcpFlags(TcpFlags::SYN),
                window_size: self.wnd,
                urgent_ptr: 0,
                options: vec![],
            },
            payload: vec![],
        })
    }

    pub fn on_segment(&mut self, seg: TcpSegment) -> Result<Option<TcpSegment>> {
        // Basic port check.
        if seg.hdr.dst_port != self.local_port || seg.hdr.src_port != self.remote_port {
            return Ok(None);
        }

        match self.state {
            TcpClientState::SynSent => self.on_syn_sent(seg),
            TcpClientState::Established => self.on_established(seg),
            TcpClientState::FinWait1 => self.on_fin_wait_1(seg),
            TcpClientState::FinWait2 => self.on_fin_wait_2(seg),
            TcpClientState::TimeWait | TcpClientState::Closed => Ok(None),
        }
    }

    fn on_syn_sent(&mut self, seg: TcpSegment) -> Result<Option<TcpSegment>> {
        // Expect SYN+ACK
        if !seg.hdr.flags.contains(TcpFlags::SYN) || !seg.hdr.flags.contains(TcpFlags::ACK) {
            return Ok(None);
        }

        // Validate ACKs our SYN.
        if seg.hdr.ack != self.snd_nxt {
            return Ok(None);
        }

        // Track remote ISN, SYN consumes 1.
        self.rcv_nxt = seg.hdr.seq.wrapping_add(1);
        self.snd_una = seg.hdr.ack;

        // Reply ACK.
        let ack_seg = TcpSegment {
            hdr: TcpHeader {
                src_port: self.local_port,
                dst_port: self.remote_port,
                seq: self.snd_nxt,
                ack: self.rcv_nxt,
                data_offset_words: 5,
                flags: TcpFlags(TcpFlags::ACK),
                window_size: self.wnd,
                urgent_ptr: 0,
                options: vec![],
            },
            payload: vec![],
        };

        self.state = TcpClientState::Established;
        Ok(Some(ack_seg))
    }

    fn on_established(&mut self, seg: TcpSegment) -> Result<Option<TcpSegment>> {
        // Pure logic: accept in-order payload and advance rcv_nxt.
        //
        // For now ignore out-of-order / retransmit.
        if seg.hdr.flags.contains(TcpFlags::RST) {
            self.state = TcpClientState::Closed;
            return Ok(None);
        }

        // ACK processing
        if seg.hdr.flags.contains(TcpFlags::ACK) {
            if seg.hdr.ack.wrapping_sub(self.snd_una) <= self.snd_nxt.wrapping_sub(self.snd_una) {
                self.snd_una = seg.hdr.ack;
            }
        }

        let mut need_ack = false;

        if !seg.payload.is_empty() {
            // Only accept exact expected seq.
            if seg.hdr.seq == self.rcv_nxt {
                self.rcv_nxt = self.rcv_nxt.wrapping_add(seg.payload.len() as u32);
                need_ack = true;
            }
        }

        if seg.hdr.flags.contains(TcpFlags::FIN) {
            // FIN consumes 1.
            if seg.hdr.seq == self.rcv_nxt {
                self.rcv_nxt = self.rcv_nxt.wrapping_add(1);
                need_ack = true;
            }
            // We don't implement full close handshake for server side; just ACK and go TIME_WAIT.
            self.state = TcpClientState::TimeWait;
        }

        if need_ack {
            return Ok(Some(self.make_ack()));
        }

        Ok(None)
    }

    fn on_fin_wait_1(&mut self, seg: TcpSegment) -> Result<Option<TcpSegment>> {
        // Wait for ACK of our FIN.
        if seg.hdr.flags.contains(TcpFlags::ACK) && seg.hdr.ack == self.snd_nxt {
            self.snd_una = seg.hdr.ack;
            self.state = TcpClientState::FinWait2;
        }
        // Also handle FIN from peer.
        if seg.hdr.flags.contains(TcpFlags::FIN) {
            if seg.hdr.seq == self.rcv_nxt {
                self.rcv_nxt = self.rcv_nxt.wrapping_add(1);
                let ack = self.make_ack();
                self.state = TcpClientState::TimeWait;
                return Ok(Some(ack));
            }
        }
        Ok(None)
    }

    fn on_fin_wait_2(&mut self, seg: TcpSegment) -> Result<Option<TcpSegment>> {
        if seg.hdr.flags.contains(TcpFlags::FIN) {
            if seg.hdr.seq == self.rcv_nxt {
                self.rcv_nxt = self.rcv_nxt.wrapping_add(1);
                let ack = self.make_ack();
                self.state = TcpClientState::TimeWait;
                return Ok(Some(ack));
            }
        }
        Ok(None)
    }

    pub fn send_data(&mut self, data: &[u8]) -> Result<TcpSegment> {
        if self.state != TcpClientState::Established {
            bail!("send_data called in state {:?}", self.state);
        }

        let seq = self.snd_nxt;
        self.snd_nxt = self.snd_nxt.wrapping_add(data.len() as u32);

        Ok(TcpSegment {
            hdr: TcpHeader {
                src_port: self.local_port,
                dst_port: self.remote_port,
                seq,
                ack: self.rcv_nxt,
                data_offset_words: 5,
                flags: TcpFlags(TcpFlags::ACK | TcpFlags::PSH),
                window_size: self.wnd,
                urgent_ptr: 0,
                options: vec![],
            },
            payload: data.to_vec(),
        })
    }

    pub fn close(&mut self) -> Result<TcpSegment> {
        if self.state != TcpClientState::Established {
            bail!("close called in state {:?}", self.state);
        }

        let seq = self.snd_nxt;
        self.snd_nxt = self.snd_nxt.wrapping_add(1);
        self.state = TcpClientState::FinWait1;

        Ok(TcpSegment {
            hdr: TcpHeader {
                src_port: self.local_port,
                dst_port: self.remote_port,
                seq,
                ack: self.rcv_nxt,
                data_offset_words: 5,
                flags: TcpFlags(TcpFlags::FIN | TcpFlags::ACK),
                window_size: self.wnd,
                urgent_ptr: 0,
                options: vec![],
            },
            payload: vec![],
        })
    }

    fn make_ack(&self) -> TcpSegment {
        TcpSegment {
            hdr: TcpHeader {
                src_port: self.local_port,
                dst_port: self.remote_port,
                seq: self.snd_nxt,
                ack: self.rcv_nxt,
                data_offset_words: 5,
                flags: TcpFlags(TcpFlags::ACK),
                window_size: self.wnd,
                urgent_ptr: 0,
                options: vec![],
            },
            payload: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(
        src_port: u16,
        dst_port: u16,
        seq: u32,
        ack: u32,
        flags: u16,
        payload: &[u8],
    ) -> TcpSegment {
        TcpSegment {
            hdr: TcpHeader {
                src_port,
                dst_port,
                seq,
                ack,
                data_offset_words: 5,
                flags: TcpFlags(flags),
                window_size: 65535,
                urgent_ptr: 0,
                options: vec![],
            },
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn handshake_and_data() {
        let mut c = TcpClient::new(40000, 10001, 100);

        // 1) SYN
        let syn = c.connect().unwrap();
        assert_eq!(c.state, TcpClientState::SynSent);
        assert!(syn.hdr.flags.contains(TcpFlags::SYN));
        assert_eq!(syn.hdr.seq, 100);

        // 2) SYN-ACK from server (server isn=9000)
        let synack = seg(
            10001,
            40000,
            9000,
            101, // ack our SYN
            TcpFlags::SYN | TcpFlags::ACK,
            &[],
        );
        let ack = c.on_segment(synack).unwrap().unwrap();
        assert_eq!(c.state, TcpClientState::Established);
        assert!(ack.hdr.flags.contains(TcpFlags::ACK));
        assert_eq!(ack.hdr.ack, 9001);
        assert_eq!(ack.hdr.seq, 101);

        // 3) Client sends PSH+ACK data
        let d = c.send_data(b"hi").unwrap();
        assert!(d.hdr.flags.contains(TcpFlags::PSH));
        assert!(d.hdr.flags.contains(TcpFlags::ACK));
        assert_eq!(d.hdr.seq, 101);

        // 4) Server ACKs data
        let a = seg(10001, 40000, 9001, 103, TcpFlags::ACK, &[]);
        assert!(c.on_segment(a).unwrap().is_none());

        // 5) Server sends data in-order
        let sdata = seg(10001, 40000, 9001, 103, TcpFlags::ACK, b"ok");
        let ack2 = c.on_segment(sdata).unwrap().unwrap();
        assert_eq!(ack2.hdr.ack, 9003);

        // 6) Close
        let fin = c.close().unwrap();
        assert_eq!(c.state, TcpClientState::FinWait1);
        assert!(fin.hdr.flags.contains(TcpFlags::FIN));

        // 7) Server ACK our FIN
        let finack = seg(10001, 40000, 9003, 104, TcpFlags::ACK, &[]);
        assert!(c.on_segment(finack).unwrap().is_none());
        assert_eq!(c.state, TcpClientState::FinWait2);

        // 8) Server FIN
        let sfin = seg(10001, 40000, 9003, 104, TcpFlags::FIN | TcpFlags::ACK, &[]);
        let last_ack = c.on_segment(sfin).unwrap().unwrap();
        assert_eq!(c.state, TcpClientState::TimeWait);
        assert!(last_ack.hdr.flags.contains(TcpFlags::ACK));
        assert_eq!(last_ack.hdr.ack, 9004);
    }
}
