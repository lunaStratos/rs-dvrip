//! 한 채널의 실시간 영상을 Annex-B 파일로 저장한다.
//!
//! ```sh
//! cargo run --example save_stream -- 192.168.1.10 34567 admin 비밀번호 0 out.h265
//! ffplay out.h265      # 또는: ffmpeg -i out.h265 -c copy out.mp4
//!
//! # 부스트림(저해상도·저지연)을 받으려면 마지막에 Extra1 을 붙인다
//! cargo run --example save_stream -- 192.168.1.10 34567 admin 비밀번호 0 out.h265 Extra1
//! ```
//!
//! Ctrl+C 로 종료한다.

use rs_dvrip::{Device, Stream};
use tokio::io::AsyncWriteExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 7 {
        eprintln!(
            "사용법: save_stream <HOST> <PORT> <USER> <PASSWORD> <CHANNEL> <OUT.h265> [Main|Extra1]"
        );
        std::process::exit(2);
    }
    let host = &args[1];
    let port: u16 = args[2].parse()?;
    let (user, pass) = (&args[3], &args[4]);
    let channel: u32 = args[5].parse()?;
    let out_path = &args[6];
    let quality = match args.get(7).map(String::as_str) {
        None | Some("Main") => Stream::Main,
        Some("Extra1") => Stream::Extra1,
        Some(other) => {
            eprintln!("알 수 없는 스트림: {other} (Main 또는 Extra1)");
            std::process::exit(2);
        }
    };

    let mut device = Device::connect((host.as_str(), port)).await?;
    let info = device.login(user, pass).await?;
    eprintln!(
        "로그인 성공 (세션 0x{:08X}, 채널 {}개)",
        info.session, info.channel_count
    );
    if channel >= info.channel_count {
        eprintln!(
            "경고: 채널 {channel} 은 이 장비의 범위(0..{})를 벗어납니다",
            info.channel_count
        );
    }

    let mut stream = device.monitor(channel, quality).await?;
    let mut file = tokio::fs::File::create(out_path).await?;
    eprintln!("채널 {channel} 수신 시작 — Ctrl+C 로 종료");

    let mut video = 0u64;
    let mut keys = 0u64;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\n중단 요청");
                break;
            }
            frame = stream.next_frame() => {
                match frame? {
                    Some(f) if f.kind.is_video() => {
                        file.write_all(&f.data).await?;
                        video += 1;
                        if f.kind == rs_dvrip::FrameKind::VideoKey {
                            keys += 1;
                        }
                        if video % 30 == 0 {
                            eprint!("\r비디오 프레임 {video} (키프레임 {keys})");
                        }
                    }
                    // 오디오 프레임은 이 예제에서 버린다
                    Some(_) => {}
                    None => {
                        eprintln!("\n장비가 연결을 닫았습니다");
                        break;
                    }
                }
            }
        }
    }

    file.flush().await?;
    eprintln!("\n저장 완료: {out_path} — 비디오 {video} 프레임(키프레임 {keys})");
    Ok(())
}
