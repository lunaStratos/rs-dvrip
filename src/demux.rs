//! XM 미디어 프레임 디먹서.
//!
//! 실시간 데이터 패킷(`msg::MONITOR_DATA`)의 본문을 이어붙이면 XM 고유 프레임
//! 헤더가 붙은 바이트열이 된다. 이 모듈은 그 헤더를 떼어 코덱 페이로드만 뽑는다.
//!
//! ```text
//! 00 00 01 FC   비디오 키프레임(I)   헤더 16바이트, 길이 = u32 LE @12
//! 00 00 01 FE   비디오 키프레임(I)   위와 동일
//! 00 00 01 FD   비디오 보간프레임(P) 헤더  8바이트, 길이 = u32 LE @4
//! 00 00 01 FA   오디오               헤더  8바이트, 길이 = u16 LE @6
//! 00 00 01 F9   오디오/기타          헤더  8바이트, 길이 = u16 LE @6
//! ```
//!
//! 비디오 프레임의 페이로드는 그대로 H.264/H.265 **Annex-B** 엘리멘터리 스트림이므로,
//! 순서대로 이어 쓰면 `ffmpeg`/`ffplay` 가 바로 재생할 수 있는 파일이 된다.

/// 프레임 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    /// 비디오 키프레임(I-frame). 여기서부터 디코딩을 시작할 수 있다.
    VideoKey,
    /// 비디오 보간 프레임(P/B-frame). 직전 키프레임이 있어야 디코딩된다.
    VideoInter,
    /// 오디오 프레임.
    Audio,
}

impl FrameKind {
    /// 비디오 프레임인지 여부.
    pub fn is_video(self) -> bool {
        matches!(self, FrameKind::VideoKey | FrameKind::VideoInter)
    }
}

/// 디먹싱된 프레임 하나.
#[derive(Debug, Clone)]
pub struct Frame {
    /// 이 프레임이 비디오 키/보간 프레임인지 오디오인지.
    pub kind: FrameKind,
    /// XM 헤더를 제거한 페이로드.
    /// 비디오면 H.264/H.265 Annex-B 바이트열이다.
    pub data: Vec<u8>,
}

/// 바이트열을 받아 프레임 단위로 잘라내는 상태 기계.
///
/// 패킷 경계와 프레임 경계는 일치하지 않으므로, 완성되지 않은 프레임은
/// 내부 버퍼에 남겨 다음 [`Demux::push`] 에서 이어 처리한다.
#[derive(Debug, Default)]
pub struct Demux {
    buf: Vec<u8>,
}

impl Demux {
    /// 빈 디먹서를 만든다.
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// 내부 버퍼에 남아 있는(미완성) 바이트 수.
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    /// `data` 를 밀어 넣고, 완성된 프레임을 `out` 에 순서대로 append 한다.
    pub fn push(&mut self, data: &[u8], out: &mut Vec<Frame>) {
        self.buf.extend_from_slice(data);

        loop {
            // 헤더를 판단할 최소 바이트가 없으면 더 기다린다.
            if self.buf.len() < 8 {
                return;
            }

            // 프레임 선두가 아니면 다음 마커까지 버리고 재동기한다.
            if !is_marker(&self.buf, 0) {
                if !self.resync() {
                    return;
                }
                continue;
            }

            let (hdr_len, data_len, kind) = match self.buf[3] {
                0xFC | 0xFE => {
                    if self.buf.len() < 16 {
                        return;
                    }
                    (
                        16usize,
                        u32_le(&self.buf[12..16]) as usize,
                        FrameKind::VideoKey,
                    )
                }
                0xFD => (
                    8usize,
                    u32_le(&self.buf[4..8]) as usize,
                    FrameKind::VideoInter,
                ),
                0xFA | 0xF9 => (8usize, u16_le(&self.buf[6..8]) as usize, FrameKind::Audio),
                _ => {
                    // 알 수 없는 마커: 1바이트 밀고 재동기
                    if !self.resync() {
                        return;
                    }
                    continue;
                }
            };

            let total = hdr_len + data_len;
            if self.buf.len() < total {
                return; // 프레임이 아직 다 안 왔다
            }

            out.push(Frame {
                kind,
                data: self.buf[hdr_len..total].to_vec(),
            });
            self.buf.drain(..total);
        }
    }

    /// 다음 마커까지 버린다. 마커를 못 찾으면 꼬리만 남기고 `false`.
    ///
    /// 꼬리는 3바이트를 남긴다. 마커(`00 00 01`)가 버퍼 끝에 걸쳐 들어온 경우
    /// 2바이트만 남기면 선두 바이트를 잃고 한 프레임이 어긋나기 때문이다.
    fn resync(&mut self) -> bool {
        match find_marker(&self.buf, 1) {
            Some(idx) => {
                self.buf.drain(..idx);
                true
            }
            None => {
                let keep = self.buf.len().saturating_sub(3);
                self.buf.drain(..keep);
                false
            }
        }
    }
}

fn is_marker(buf: &[u8], i: usize) -> bool {
    buf.len() >= i + 4 && buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1
}

/// `start` 이후의 `00 00 01` 마커 위치(그리고 종류 바이트까지 존재하는 곳)를 찾는다.
fn find_marker(buf: &[u8], start: usize) -> Option<usize> {
    (start..buf.len().saturating_sub(3))
        .find(|&i| buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1)
}

fn u32_le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn u16_le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16바이트 헤더 키프레임 하나를 만든다.
    fn key_frame(payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0u8; 16];
        v[0..4].copy_from_slice(&[0x00, 0x00, 0x01, 0xFC]);
        v[12..16].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    /// 8바이트 헤더 P-프레임 하나를 만든다.
    fn inter_frame(payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0u8; 8];
        v[0..4].copy_from_slice(&[0x00, 0x00, 0x01, 0xFD]);
        v[4..8].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn audio_frame(payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0u8; 8];
        v[0..4].copy_from_slice(&[0x00, 0x00, 0x01, 0xFA]);
        v[6..8].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn extracts_key_frame_payload() {
        let mut d = Demux::new();
        let mut out = Vec::new();
        d.push(&key_frame(&[0xAA, 0xBB, 0xCC, 0xDD]), &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, FrameKind::VideoKey);
        assert_eq!(out[0].data, vec![0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(d.buffered(), 0);
    }

    #[test]
    fn extracts_mixed_frame_kinds_in_order() {
        let mut d = Demux::new();
        let mut out = Vec::new();
        let mut wire = key_frame(&[1, 2, 3]);
        wire.extend(inter_frame(&[4, 5]));
        wire.extend(audio_frame(&[6]));
        wire.extend(inter_frame(&[7, 8, 9]));
        d.push(&wire, &mut out);

        let kinds: Vec<_> = out.iter().map(|f| f.kind).collect();
        assert_eq!(
            kinds,
            vec![
                FrameKind::VideoKey,
                FrameKind::VideoInter,
                FrameKind::Audio,
                FrameKind::VideoInter
            ]
        );
        assert_eq!(out[3].data, vec![7, 8, 9]);
    }

    /// 패킷 경계가 프레임 중간을 가르더라도 복원돼야 한다.
    #[test]
    fn reassembles_frames_split_across_pushes() {
        let wire = {
            let mut w = key_frame(&[1, 2, 3, 4, 5]);
            w.extend(inter_frame(&[6, 7, 8]));
            w
        };

        for chunk in 1..=wire.len() {
            let mut d = Demux::new();
            let mut out = Vec::new();
            for part in wire.chunks(chunk) {
                d.push(part, &mut out);
            }
            assert_eq!(out.len(), 2, "chunk={chunk}");
            assert_eq!(out[0].data, vec![1, 2, 3, 4, 5], "chunk={chunk}");
            assert_eq!(out[1].data, vec![6, 7, 8], "chunk={chunk}");
        }
    }

    /// 선두에 쓰레기 바이트가 있어도 다음 마커에서 다시 동기를 잡는다.
    #[test]
    fn resyncs_past_leading_garbage() {
        let mut d = Demux::new();
        let mut out = Vec::new();
        let mut wire = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x11, 0x22, 0x33, 0x44];
        wire.extend(key_frame(&[9, 9]));
        d.push(&wire, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].data, vec![9, 9]);
    }

    #[test]
    fn keeps_incomplete_frame_buffered() {
        let mut d = Demux::new();
        let mut out = Vec::new();
        let full = key_frame(&[1, 2, 3, 4]);
        d.push(&full[..full.len() - 2], &mut out);
        assert!(out.is_empty());
        assert!(d.buffered() > 0);
        d.push(&full[full.len() - 2..], &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].data, vec![1, 2, 3, 4]);
    }
}
