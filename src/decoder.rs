use crate::reading::Reading;

/// Incremental decoder that reassembles the meter's fixed-size frames
/// from an arbitrarily chunked byte stream.
///
/// Transports deliver bytes with no alignment guarantees; the decoder
/// scans for the sync header and yields only frames whose checksum
/// validates. A corrupted or truncated frame is skipped one byte at a
/// time, so a genuine frame embedded after a false or damaged sync is
/// still found.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends received bytes to the decoder.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Returns the next validated frame, discarding any bytes that do
    /// not begin one. Returns `None` until a full valid frame is
    /// buffered.
    pub fn next_frame(&mut self) -> Option<[u8; Reading::N_BYTES]> {
        loop {
            let Some(start) = self
                .buf
                .windows(Reading::N_SYNC_BYTES)
                .position(|w| w == Reading::SYNC)
            else {
                // No sync found; keep only a partial-sync tail.
                let keep_from = self.buf.len().saturating_sub(Reading::N_SYNC_BYTES - 1);
                self.buf.drain(..keep_from);
                return None;
            };
            self.buf.drain(..start);
            if self.buf.len() < Reading::N_BYTES {
                return None;
            }
            let frame: [u8; Reading::N_BYTES] = self.buf[..Reading::N_BYTES].try_into().unwrap();
            if Reading::validate_frame(&frame) {
                self.buf.drain(..Reading::N_BYTES);
                return Some(frame);
            }
            // Bad candidate (corruption or a false sync): advance past
            // the first sync byte and rescan.
            self.buf.drain(..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reading::tests::fix_checksum;

    fn test_frame() -> [u8; Reading::N_BYTES] {
        let mut frame = [0u8; Reading::N_BYTES];
        frame[..Reading::N_SYNC_BYTES].copy_from_slice(&Reading::SYNC);
        frame[Reading::N_BYTES - 4] = 0xee;
        fix_checksum(&mut frame);
        frame
    }

    #[test]
    fn test_whole_frame() {
        let mut decoder = FrameDecoder::new();
        decoder.push(&test_frame());
        assert_eq!(decoder.next_frame(), Some(test_frame()));
        assert_eq!(decoder.next_frame(), None);
    }

    #[test]
    fn test_split_frame() {
        let mut decoder = FrameDecoder::new();
        let frame = test_frame();
        decoder.push(&frame[..20]);
        assert_eq!(decoder.next_frame(), None);
        decoder.push(&frame[20..]);
        assert_eq!(decoder.next_frame(), Some(frame));
    }

    #[test]
    fn test_garbage_before_sync() {
        let mut decoder = FrameDecoder::new();
        decoder.push(&[0x00, 0xaa, 0x55, 0x12]);
        decoder.push(&test_frame());
        assert_eq!(decoder.next_frame(), Some(test_frame()));
    }

    #[test]
    fn test_two_frames_in_one_chunk() {
        let mut decoder = FrameDecoder::new();
        let mut bytes = test_frame().to_vec();
        bytes.extend_from_slice(&test_frame());
        decoder.push(&bytes);
        assert_eq!(decoder.next_frame(), Some(test_frame()));
        assert_eq!(decoder.next_frame(), Some(test_frame()));
        assert_eq!(decoder.next_frame(), None);
    }

    #[test]
    fn test_garbage_only_is_discarded() {
        let mut decoder = FrameDecoder::new();
        decoder.push(&[0x12; 1024]);
        assert_eq!(decoder.next_frame(), None);
        // Buffer must not grow without bound on garbage input.
        assert!(decoder.buf.len() < Reading::N_SYNC_BYTES);
    }

    #[test]
    fn test_sync_split_across_chunks() {
        let mut decoder = FrameDecoder::new();
        let frame = test_frame();
        decoder.push(&[0x99]);
        decoder.push(&frame[..2]);
        assert_eq!(decoder.next_frame(), None);
        decoder.push(&frame[2..]);
        assert_eq!(decoder.next_frame(), Some(frame));
    }

    #[test]
    fn test_truncated_frame_does_not_swallow_next() {
        // Frame A loses its last 6 bytes in transit; frame B arrives
        // complete. B's sync falls inside what a naive decoder would
        // consume as A's payload; the decoder must still yield B.
        let mut decoder = FrameDecoder::new();
        let frame = test_frame();
        decoder.push(&frame[..Reading::N_BYTES - 6]);
        decoder.push(&frame);
        assert_eq!(decoder.next_frame(), Some(frame));
        assert_eq!(decoder.next_frame(), None);
    }

    #[test]
    fn test_corrupted_frame_is_skipped() {
        let mut decoder = FrameDecoder::new();
        let mut corrupted = test_frame();
        corrupted[10] ^= 0x01;
        decoder.push(&corrupted);
        decoder.push(&test_frame());
        assert_eq!(decoder.next_frame(), Some(test_frame()));
        assert_eq!(decoder.next_frame(), None);
    }

    #[test]
    fn test_false_sync_inside_garbage() {
        // A sync pattern appears in noise with no valid frame behind
        // it, followed by a real frame.
        let mut decoder = FrameDecoder::new();
        let mut noise = Reading::SYNC.to_vec();
        noise.extend_from_slice(&[0x5a; 51]);
        decoder.push(&noise);
        decoder.push(&test_frame());
        assert_eq!(decoder.next_frame(), Some(test_frame()));
    }
}
