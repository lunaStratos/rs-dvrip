use thiserror::Error;

/// `rs-dvrip` 의 모든 오류.
#[derive(Debug, Error)]
pub enum Error {
    /// 소켓/파일 I/O 오류. 연결이 끊기면 `UnexpectedEof` 로 나타난다.
    #[error("I/O 오류: {0}")]
    Io(#[from] std::io::Error),

    /// 제어 메시지(JSON) 직렬화·파싱 오류.
    #[error("JSON 오류: {0}")]
    Json(#[from] serde_json::Error),

    /// DVRIP 응답은 항상 0xFF 로 시작한다. 다른 값이면 DVRIP 장비가 아니거나
    /// 스트림 동기가 어긋난 상태다.
    #[error("DVRIP 응답이 아님 (선두 바이트 0x{0:02x})")]
    BadMagic(u8),

    /// 장비가 로그인을 거부했다. `Ret` 100 이 성공이다.
    #[error("로그인 거부 (Ret={0}) — 아이디/비밀번호 또는 암호화 방식 확인 필요")]
    LoginRejected(i64),

    /// 로그인 응답의 `SessionID` 가 `"0x...."` 16진 문자열이 아니었다.
    #[error("SessionID 파싱 실패: {0:?}")]
    BadSessionId(String),

    /// 지정한 단계에서 응답이 오지 않았다.
    /// 흔한 원인: 장비 무응답, 동시접속 슬롯 소진, 권한 부족.
    #[error("타임아웃: {0}")]
    Timeout(&'static str),
}

/// [`enum@Error`] 를 기본 오류로 쓰는 결과 타입.
pub type Result<T> = std::result::Result<T, Error>;
