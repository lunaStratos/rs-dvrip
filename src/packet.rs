//! DVRIP 와이어 포맷: 20바이트 헤더 + 본문(JSON 또는 바이너리 미디어).
//!
//! ```text
//! 오프셋  크기  내용
//!   0      1   0xFF (고정)
//!   1      1   버전 (0)
//!   2      2   예약 (0)
//!   4      4   SessionID   (u32, little-endian)
//!   8      4   시퀀스 번호 (u32, little-endian)
//!  12      1   전체 패킷 수
//!  13      1   현재 패킷 번호
//!  14      2   메시지 ID   (u16, little-endian)
//!  16      4   본문 길이   (u32, little-endian)
//!  20      N   본문 — JSON 요청/응답은 뒤에 0x0A 0x00 이 붙고 길이에 포함된다
//! ```

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::{Error, Result};

/// 헤더 길이(바이트).
pub const HEADER_LEN: usize = 20;

/// DVRIP 메시지 ID.
pub mod msg {
    /// 로그인 요청.
    pub const LOGIN_REQ: u16 = 1000;
    /// 로그인 응답.
    pub const LOGIN_RESP: u16 = 1001;
    /// 세션 유지(heartbeat).
    pub const KEEPALIVE: u16 = 1006;
    /// 실시간 영상 데이터.
    pub const MONITOR_DATA: u16 = 1412;
    /// OPMonitor Start — 실시간 전송 시작.
    pub const MONITOR_START: u16 = 1410;
    /// OPMonitor Claim — 실시간 채널 점유.
    pub const MONITOR_CLAIM: u16 = 1413;
}

/// 헤더 + 본문을 하나의 버퍼로 인코딩한다.
///
/// `json` 은 JSON 바이트이며, DVRIP 규약에 따라 뒤에 `0x0A 0x00` 을 붙이고
/// 그 2바이트까지 길이에 포함시킨다.
pub fn encode(session: u32, seq: u32, msg_id: u16, json: &[u8]) -> Vec<u8> {
    let body_len = json.len() + 2;
    let mut out = Vec::with_capacity(HEADER_LEN + body_len);
    out.resize(HEADER_LEN, 0);
    out[0] = 0xFF;
    out[4..8].copy_from_slice(&session.to_le_bytes());
    out[8..12].copy_from_slice(&seq.to_le_bytes());
    out[14..16].copy_from_slice(&msg_id.to_le_bytes());
    out[16..20].copy_from_slice(&(body_len as u32).to_le_bytes());
    out.extend_from_slice(json);
    out.push(0x0A);
    out.push(0x00);
    out
}

/// 패킷 하나를 전송한다.
pub async fn write_packet<W>(
    w: &mut W,
    session: u32,
    seq: u32,
    msg_id: u16,
    json: &[u8],
) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let buf = encode(session, seq, msg_id, json);
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

/// 패킷 하나를 수신해 `(메시지 ID, 본문)` 을 돌려준다.
///
/// 본문은 그대로 반환하므로, JSON 응답이면 뒤의 `0x0A 0x00` 이 남아 있다.
/// [`trim_json`] 으로 다듬어 파싱한다.
pub async fn read_packet<R>(r: &mut R) -> Result<(u16, Vec<u8>)>
where
    R: AsyncRead + Unpin,
{
    let mut hdr = [0u8; HEADER_LEN];
    r.read_exact(&mut hdr).await?;
    if hdr[0] != 0xFF {
        return Err(Error::BadMagic(hdr[0]));
    }
    let msg_id = u16::from_le_bytes([hdr[14], hdr[15]]);
    let len = u32::from_le_bytes([hdr[16], hdr[17], hdr[18], hdr[19]]) as usize;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    Ok((msg_id, body))
}

/// JSON 본문 뒤에 붙은 NUL·개행·공백을 떼어낸다.
pub fn trim_json(body: &[u8]) -> &[u8] {
    let mut end = body.len();
    while end > 0 && matches!(body[end - 1], 0 | b'\n' | b'\r' | b' ' | b'\t') {
        end -= 1;
    }
    &body[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_header_and_terminates_body() {
        let pkt = encode(0x0000000C, 3, msg::LOGIN_REQ, b"{}");
        assert_eq!(pkt[0], 0xFF);
        assert_eq!(&pkt[4..8], &0x0000000Cu32.to_le_bytes());
        assert_eq!(&pkt[8..12], &3u32.to_le_bytes());
        assert_eq!(&pkt[14..16], &1000u16.to_le_bytes());
        // 본문 길이는 "{}" 2바이트 + 종결 2바이트 = 4
        assert_eq!(&pkt[16..20], &4u32.to_le_bytes());
        assert_eq!(&pkt[HEADER_LEN..], b"{}\x0a\x00");
    }

    #[tokio::test]
    async fn round_trips_through_reader() {
        let pkt = encode(1, 2, msg::KEEPALIVE, b"{\"Name\":\"KeepAlive\"}");
        let mut cursor = std::io::Cursor::new(pkt);
        let (id, body) = read_packet(&mut cursor).await.unwrap();
        assert_eq!(id, msg::KEEPALIVE);
        assert_eq!(trim_json(&body), b"{\"Name\":\"KeepAlive\"}");
    }

    #[tokio::test]
    async fn rejects_non_dvrip_magic() {
        let mut bad = vec![0u8; HEADER_LEN];
        bad[0] = 0x16; // 예: TLS 핸드셰이크
        let mut cursor = std::io::Cursor::new(bad);
        match read_packet(&mut cursor).await {
            Err(Error::BadMagic(0x16)) => {}
            other => panic!("expected BadMagic, got {other:?}"),
        }
    }
}
