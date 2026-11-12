# rs-dvrip

[![crates.io](https://img.shields.io/crates/v/rs-dvrip.svg)](https://crates.io/crates/rs-dvrip)
[![docs.rs](https://docs.rs/rs-dvrip/badge.svg)](https://docs.rs/rs-dvrip)
[![license](https://img.shields.io/crates/l/rs-dvrip.svg)](#라이선스)

Xiongmai(XM) 계열 DVR/NVR/IP 카메라가 쓰는 **DVRIP**(일명 "Sofia", NetSurveillance)
프로토콜의 async Rust 클라이언트.

XMEye · 미라클 · 이지피스 등 다수의 중국산 CCTV 장비가 이 프로토콜을 쓴다.
RTSP 가 비활성화됐거나 아예 지원되지 않는 장비에서도 실시간 영상을 받아올 수 있다.

```rust
use rs_dvrip::{Device, Stream};

let mut device = Device::connect(("192.168.1.10", 34567)).await?;
let info = device.login("admin", "비밀번호").await?;

// 채널 수는 로그인 응답에서 알려준다 — 하드코딩할 필요가 없다
println!("{:?} / {}채널", info.device_type, info.channel_count);

let mut stream = device.monitor(0, Stream::Main).await?;
while let Some(frame) = stream.next_frame().await? {
    if frame.kind.is_video() {
        // frame.data = H.264/H.265 Annex-B
    }
}
```

## 범위

이 크레이트는 **프로토콜만** 다룬다. 영상을 디코딩하지 않고 H.264/H.265
**Annex-B** 엘리멘터리 스트림을 그대로 넘겨주므로, 재생·트랜스코딩은 `ffmpeg`
등 원하는 수단에 맡기면 된다. 덕분에 의존성이 작고 용도를 가리지 않는다.

| 기능 | 상태 |
|------|------|
| 로그인 (XM "Sofia" 비밀번호 해시) | ✅ |
| 장비 구성 탐지 (채널 수·장비 종류) | ✅ |
| 세션 유지 (KeepAlive 자동 전송) | ✅ |
| 실시간 스트림 (OPMonitor Claim → Start) | ✅ |
| XM 프레임 디먹싱 (키/보간/오디오 분리) | ✅ |
| 녹화 재생(OPPlayBack), PTZ, 설정 조회 | ❌ 아직 |

## 설치

```toml
[dependencies]
rs-dvrip = "0.1"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

의존성은 `tokio`, `serde_json`, `md-5`, `thiserror` 뿐이다. `unsafe` 코드는 없다
(`#![forbid(unsafe_code)]`).

## 예제

```sh
# 로그인만 해보고 장비 정보 출력
cargo run --example probe -- 192.168.1.10 34567 admin 비밀번호

# 채널 0 을 Annex-B 파일로 저장 (Ctrl+C 로 종료)
cargo run --example save_stream -- 192.168.1.10 34567 admin 비밀번호 0 out.h265
ffplay out.h265
ffmpeg -i out.h265 -c copy out.mp4
```

## 알아두면 좋은 것

- **연결 하나당 채널 하나.** DVRIP 는 한 TCP 연결에서 한 채널만 전송한다.
  4채널을 동시에 보려면 `Device` 를 4개 만들면 된다.
- **채널 수는 로그인 응답에서 온다.** 별도 조회가 필요 없다. 응답의 `ChannelNum`
  필드가 `DeviceInfo::channel_count` 로 올라오고, `info.channels()` 로 유효한
  채널 번호(`0..channel_count`)를 순회할 수 있다. 채널마다 해상도가 다를 수 있다
  (실측 예: CH1 만 2560×1440, 나머지 1920×1080).
- **포트는 장비마다 다르다.** 기본은 `34567` 이지만 `8000`/`8001` 로 바뀐 장비도 많다.
- **키프레임 간격이 길 수 있다.** 주 스트림의 GOP 가 7초쯤 되는 장비도 있어,
  첫 화면이 늦게 뜨는 게 정상일 수 있다. 저지연이 필요하면 `Stream::Extra1` 을 쓴다.
- **동시 스트림 슬롯이 제한적이다.** 슬롯이 소진되면 연결은 되는데 데이터가 오지
  않으므로, 첫 프레임에 타임아웃을 걸어 `Error::Timeout` 으로 알려준다.

## 프로토콜 요약

20바이트 리틀엔디안 헤더 + 본문(JSON 또는 바이너리 미디어):

```text
오프셋  크기  내용
  0      1   0xFF (고정)
  1      1   버전
  2      2   예약
  4      4   SessionID
  8      4   시퀀스 번호
 12      1   전체 패킷 수
 13      1   현재 패킷 번호
 14      2   메시지 ID
 16      4   본문 길이
 20      N   본문 — JSON 이면 뒤에 0x0A 0x00 (길이에 포함)
```

주요 메시지 ID: 로그인 `1000`/`1001`, KeepAlive `1006`,
OPMonitor Start `1410`, Claim `1413`, 실시간 데이터 `1412`.

비밀번호는 표준 MD5 가 아니라 XM 고유 해시다. MD5 16바이트를 2바이트씩 더해
62진 문자 8자로 줄인다 (`sofia_hash`). 빈 비밀번호는 잘 알려진 `tlJwpbo6` 가 된다.

실시간 데이터 본문을 이어붙이면 XM 프레임 헤더가 붙은 바이트열이 되고,
이를 떼어내면 Annex-B 가 남는다:

```text
00 00 01 FC / FE   비디오 키프레임     헤더 16B, 길이 u32 LE @12
00 00 01 FD        비디오 보간프레임   헤더  8B, 길이 u32 LE @4
00 00 01 FA / F9   오디오              헤더  8B, 길이 u16 LE @6
```

## 보안 / 법적 주의

DVRIP 는 평문 프로토콜이고 비밀번호 해시도 재전송 공격에 취약하다.
신뢰할 수 있는 망에서만 쓰고, 장비를 인터넷에 직접 노출하지 않는 게 좋다.

**접근 권한이 있는 장비에만 사용할 것.** 이 코드는 상호운용과 자신의 장비
제어를 위한 것이며, 권한 없는 접근에 쓰는 것은 대부분의 국가에서 불법이다.

## 라이선스

[MIT](LICENSE-MIT) 또는 [Apache-2.0](LICENSE-APACHE) 중 선택.

기여물은 별도 명시가 없으면 위 두 라이선스로 동일하게 제공되는 것으로 본다.
