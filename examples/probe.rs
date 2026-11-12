//! 접속·로그인만 해보고 장비 정보를 출력한다.
//!
//! ```sh
//! cargo run --example probe -- 192.168.1.10 34567 admin 비밀번호
//! ```

use rs_dvrip::Device;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("사용법: probe <HOST> <PORT> <USER> <PASSWORD>");
        std::process::exit(2);
    }
    let (host, port, user, pass) = (&args[1], args[2].parse::<u16>()?, &args[3], &args[4]);

    println!("접속 중: {host}:{port}");
    let mut device = Device::connect((host.as_str(), port)).await?;
    let info = device.login(user, pass).await?;

    println!("로그인 성공");
    println!("  SessionID     : 0x{:08X}", info.session);
    println!("  KeepAlive     : {:?}", info.alive_interval);
    println!(
        "  장비 종류     : {}",
        info.device_type.as_deref().unwrap_or("(알 수 없음)")
    );
    println!("  채널 수       : {}", info.channel_count);
    println!("  부가 채널     : {}", info.extra_channels);
    println!(
        "  채널 번호     : {}",
        info.channels()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  응답 원본     : {}",
        serde_json::to_string_pretty(&info.raw)?
    );
    Ok(())
}
