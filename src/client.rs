//! 접속 · 로그인 · 실시간 스트림.

use std::time::Duration;

use serde_json::{json, Value};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpStream, ToSocketAddrs};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::demux::{Demux, Frame};
use crate::error::{Error, Result};
use crate::hash::sofia_hash;
use crate::packet::{msg, read_packet, trim_json, write_packet};

/// 접속 타임아웃.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 로그인 응답 타임아웃.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(8);
/// OPMonitor Claim 응답 타임아웃(응답이 없어도 계속 진행한다).
const CLAIM_TIMEOUT: Duration = Duration::from_secs(5);
/// 스트림 시작 후 첫 데이터까지의 타임아웃.
const FIRST_DATA_TIMEOUT: Duration = Duration::from_secs(10);

/// 요청할 스트림 품질.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stream {
    /// 주 스트림 — 고화질, 키프레임 간격이 길 수 있다.
    #[default]
    Main,
    /// 부 스트림 — 저해상도·저지연.
    Extra1,
}

impl Stream {
    /// DVRIP `StreamType` 값.
    pub fn as_str(self) -> &'static str {
        match self {
            Stream::Main => "Main",
            Stream::Extra1 => "Extra1",
        }
    }
}

/// 로그인 성공 시 장비가 알려주는 정보.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    /// 이후 모든 요청에 실어 보내는 세션 ID.
    pub session: u32,
    /// 장비가 요구하는 KeepAlive 주기.
    pub alive_interval: Duration,
    /// 이 장비의 영상 채널 수(로그인 응답의 `ChannelNum`).
    ///
    /// 유효한 채널 번호는 `0..channel_count` 다. 응답에 값이 없는 장비도 있어
    /// 그 경우 `1` 로 둔다. [`DeviceInfo::channels`] 로 순회할 수 있다.
    pub channel_count: u32,
    /// 부가 채널 수(로그인 응답의 `ExtraChannel`). 보통 0 이다.
    pub extra_channels: u32,
    /// 장비 종류 문자열(예: `"HVR"`, `"DVR"`). 응답에 없으면 `None`.
    pub device_type: Option<String>,
    /// 로그인 응답 원본 JSON. 위 필드 외의 장비별 추가 정보가 들어 있다.
    pub raw: Value,
}

impl DeviceInfo {
    /// 이 장비의 유효한 채널 번호를 순회한다.
    ///
    /// ```no_run
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// # let mut device = rs_dvrip::Device::connect(("10.0.0.1", 34567)).await?;
    /// let info = device.login("admin", "pw").await?;
    /// for ch in info.channels() {
    ///     println!("채널 {ch}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn channels(&self) -> std::ops::Range<u32> {
        0..self.channel_count
    }
}

/// DVRIP 장비와의 연결 하나.
///
/// DVRIP 는 **연결 하나당 채널 하나**를 실시간 전송한다. 여러 채널을 동시에 보려면
/// 채널 수만큼 [`Device`] 를 만들면 된다.
#[derive(Debug)]
pub struct Device {
    stream: TcpStream,
    session: u32,
    seq: u32,
    alive_interval: Duration,
}

impl Device {
    /// TCP 로 접속한다. 아직 로그인하지 않은 상태다.
    pub async fn connect<A: ToSocketAddrs>(addr: A) -> Result<Self> {
        let stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
            .await
            .map_err(|_| Error::Timeout("connect"))??;
        let _ = stream.set_nodelay(true);
        Ok(Self {
            stream,
            session: 0,
            seq: 0,
            alive_interval: Duration::from_secs(20),
        })
    }

    /// 현재 세션 ID (로그인 전에는 0).
    pub fn session(&self) -> u32 {
        self.session
    }

    /// 로그인한다. 비밀번호는 [`sofia_hash`] 로 변환해 보낸다.
    pub async fn login(&mut self, user: &str, password: &str) -> Result<DeviceInfo> {
        let req = json!({
            "EncryptType": "MD5",
            "LoginType": "DVRIP-Web",
            "UserName": user,
            "PassWord": sofia_hash(password),
        });
        self.send(msg::LOGIN_REQ, &req).await?;

        let (_, body) = timeout(LOGIN_TIMEOUT, read_packet(&mut self.stream))
            .await
            .map_err(|_| Error::Timeout("login — 장비 무응답(동시접속 슬롯 소진 가능)"))??;

        let resp: Value = serde_json::from_slice(trim_json(&body))?;
        let ret = resp.get("Ret").and_then(Value::as_i64).unwrap_or(-1);
        if ret != 100 {
            return Err(Error::LoginRejected(ret));
        }

        // SessionID 는 "0x0000000c" 형태의 16진 문자열이다.
        let sid_str = resp
            .get("SessionID")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let sid = u32::from_str_radix(
            sid_str.trim_start_matches("0x").trim_start_matches("0X"),
            16,
        )
        .map_err(|_| Error::BadSessionId(sid_str.to_string()))?;
        self.session = sid;

        let alive = resp
            .get("AliveInterval")
            .and_then(Value::as_i64)
            .filter(|s| *s > 0)
            .unwrap_or(20);
        self.alive_interval = Duration::from_secs(alive as u64);

        // 장비는 로그인 응답에서 채널 구성을 함께 알려준다.
        // 필드 이름 뒤에 공백이 붙은 장비도 있어(예: "DeviceType ") 양쪽을 본다.
        let field = |name: &str| -> Option<&Value> {
            resp.get(name)
                .or_else(|| resp.get(format!("{name} ").as_str()))
        };
        let channel_count = field("ChannelNum")
            .and_then(Value::as_u64)
            .filter(|n| *n > 0)
            .unwrap_or(1) as u32;
        let extra_channels = field("ExtraChannel").and_then(Value::as_u64).unwrap_or(0) as u32;
        let device_type = field("DeviceType")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string());

        Ok(DeviceInfo {
            session: sid,
            alive_interval: self.alive_interval,
            channel_count,
            extra_channels,
            device_type,
            raw: resp,
        })
    }

    /// 한 채널의 실시간 전송을 시작하고 프레임 스트림을 돌려준다.
    ///
    /// `Device` 를 소비한다: 이후 이 연결은 영상 수신과 KeepAlive 전용이 된다.
    /// KeepAlive 는 백그라운드 태스크가 보내며, 반환된 [`MonitorStream`] 이
    /// drop 되면 함께 정리된다.
    pub async fn monitor(mut self, channel: u32, stream: Stream) -> Result<MonitorStream> {
        let params = json!({
            "Channel": channel,
            "CombinMode": "NONE",
            "StreamType": stream.as_str(),
            "TransMode": "TCP",
            "Value": 0,
        });
        let session_hex = format!("0x{:08X}", self.session);

        // Claim → 채널 점유
        self.send(
            msg::MONITOR_CLAIM,
            &json!({
                "Name": "OPMonitor",
                "SessionID": session_hex,
                "OPMonitor": { "Action": "Claim", "Parameter": params },
            }),
        )
        .await?;
        // Claim 응답은 장비마다 생략되기도 하므로 실패를 무시한다.
        let _ = timeout(CLAIM_TIMEOUT, read_packet(&mut self.stream)).await;

        // Start → 전송 개시
        self.send(
            msg::MONITOR_START,
            &json!({
                "Name": "OPMonitor",
                "SessionID": session_hex,
                "OPMonitor": { "Action": "Start", "Parameter": params },
            }),
        )
        .await?;

        let session = self.session;
        let seq = self.seq;
        let interval = self.alive_interval;
        let (reader, writer) = self.stream.into_split();

        let keepalive = tokio::spawn(keepalive_loop(writer, session, seq, interval));

        Ok(MonitorStream {
            reader,
            demux: Demux::new(),
            pending: std::collections::VecDeque::new(),
            keepalive,
            awaiting_first: true,
        })
    }

    async fn send(&mut self, msg_id: u16, payload: &Value) -> Result<()> {
        let json = serde_json::to_vec(payload)?;
        write_packet(&mut self.stream, self.session, self.seq, msg_id, &json).await?;
        self.seq = self.seq.wrapping_add(1);
        Ok(())
    }
}

/// 세션이 끊기지 않도록 주기적으로 KeepAlive 를 보낸다.
async fn keepalive_loop(
    mut writer: OwnedWriteHalf,
    session: u32,
    mut seq: u32,
    interval: Duration,
) {
    let body = match serde_json::to_vec(&json!({
        "Name": "KeepAlive",
        "SessionID": format!("0x{session:08X}"),
    })) {
        Ok(b) => b,
        Err(_) => return,
    };

    let mut ticker = tokio::time::interval(interval);
    ticker.tick().await; // 첫 tick 은 즉시 반환되므로 버린다

    loop {
        ticker.tick().await;
        if write_packet(&mut writer, session, seq, msg::KEEPALIVE, &body)
            .await
            .is_err()
        {
            return; // 연결이 닫혔다
        }
        seq = seq.wrapping_add(1);
    }
}

/// 한 채널의 실시간 프레임 스트림.
///
/// [`MonitorStream::next_frame`] 를 반복 호출해 프레임을 받는다.
/// drop 하면 KeepAlive 태스크가 중단되고 연결이 닫힌다.
#[derive(Debug)]
pub struct MonitorStream {
    reader: OwnedReadHalf,
    demux: Demux,
    pending: std::collections::VecDeque<Frame>,
    keepalive: JoinHandle<()>,
    awaiting_first: bool,
}

impl MonitorStream {
    /// 다음 프레임을 받는다. 장비가 연결을 닫으면 `Ok(None)`.
    ///
    /// 첫 프레임에는 타임아웃이 걸려 있다(기본 10초). 장비의 동시 스트림 슬롯이
    /// 소진되면 연결은 유지되는데 데이터가 오지 않으므로, 무한 대기를 막는다.
    pub async fn next_frame(&mut self) -> Result<Option<Frame>> {
        loop {
            if let Some(frame) = self.pending.pop_front() {
                return Ok(Some(frame));
            }

            let read = read_packet(&mut self.reader);
            let result = if self.awaiting_first {
                match timeout(FIRST_DATA_TIMEOUT, read).await {
                    Ok(r) => r,
                    Err(_) => {
                        return Err(Error::Timeout(
                            "첫 영상 데이터 없음 — 스트림 슬롯 소진/권한 부족 가능",
                        ))
                    }
                }
            } else {
                read.await
            };

            let (msg_id, body) = match result {
                Ok(v) => v,
                // 장비가 정상적으로 연결을 닫은 경우
                Err(Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
                    ) =>
                {
                    return Ok(None)
                }
                Err(e) => return Err(e),
            };

            if msg_id == msg::MONITOR_DATA {
                self.awaiting_first = false;
                let mut frames = Vec::new();
                self.demux.push(&body, &mut frames);
                self.pending.extend(frames);
            }
        }
    }
}

impl Drop for MonitorStream {
    fn drop(&mut self) {
        self.keepalive.abort();
    }
}
