//! # rs-dvrip
//!
//! Xiongmai(XM) 계열 DVR/NVR/IP 카메라가 쓰는 **DVRIP**("Sofia", NetSurveillance)
//! 프로토콜의 async Rust 클라이언트.
//!
//! XMEye · 미라클 · 이지피스 등 다수의 중국산 CCTV 장비가 이 프로토콜을 쓴다.
//! RTSP 가 막혀 있거나 지원되지 않는 장비에서도 실시간 영상을 받아올 수 있다.
//!
//! ## 범위
//!
//! 이 크레이트는 **프로토콜만** 다룬다. 영상 디코딩은 하지 않고,
//! H.264/H.265 **Annex-B** 엘리멘터리 스트림을 그대로 넘겨준다.
//! 디코딩·재생은 `ffmpeg` 등 원하는 수단에 맡기면 된다.
//!
//! - 로그인 (XM "Sofia" 비밀번호 해시)
//! - 세션 유지 (KeepAlive 자동 전송)
//! - 실시간 스트림 (OPMonitor Claim → Start)
//! - XM 프레임 디먹싱 → 키/보간/오디오 프레임 분리
//!
//! ## 예시
//!
//! ```no_run
//! use rs_dvrip::{Device, Stream};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let mut device = Device::connect(("192.168.1.10", 34567)).await?;
//! let info = device.login("admin", "비밀번호").await?;
//! // 채널 수는 로그인 응답이 알려준다: info.channels() == 0..channel_count
//! println!("세션 0x{:08X} / {}채널", info.session, info.channel_count);
//!
//! // 연결 하나당 채널 하나. 모든 채널을 보려면 Device 를 채널 수만큼 만든다.
//! let mut stream = device.monitor(0, Stream::Main).await?;
//! while let Some(frame) = stream.next_frame().await? {
//!     if frame.kind.is_video() {
//!         // frame.data 는 H.264/H.265 Annex-B — 이어 쓰면 재생 가능한 파일이 된다
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## 와이어 포맷
//!
//! 20바이트 헤더 + 본문 구조는 [`packet`] 모듈 문서에,
//! XM 미디어 프레임 헤더는 [`demux`] 모듈 문서에 정리돼 있다.
//!
//! ## 주의
//!
//! 접근 권한이 있는 장비에만 사용할 것. DVRIP 는 평문 프로토콜이며
//! 비밀번호 해시도 재전송 공격에 취약하므로 신뢰할 수 있는 망에서만 쓰는 게 좋다.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![doc(html_no_source)]

mod client;
mod error;
mod hash;

pub mod demux;
pub mod packet;

pub use client::{Device, DeviceInfo, MonitorStream, Stream};
pub use demux::{Demux, Frame, FrameKind};
pub use error::{Error, Result};
pub use hash::sofia_hash;
