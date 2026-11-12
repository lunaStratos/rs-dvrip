use md5::{Digest, Md5};

/// 해시 문자에 쓰이는 62진 표.
const TBL: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Xiongmai "Sofia" 비밀번호 해시.
///
/// MD5(비밀번호) 16바이트를 2바이트씩 더해 62진 문자 8자로 줄인다.
/// 표준 MD5 가 아니라 XM 고유 방식이며, 로그인 요청의 `PassWord` 필드에 쓰인다.
///
/// ```
/// // 빈 비밀번호는 잘 알려진 기본 해시가 된다.
/// assert_eq!(rs_dvrip::sofia_hash(""), "tlJwpbo6");
/// ```
pub fn sofia_hash(password: &str) -> String {
    let digest = Md5::digest(password.as_bytes());
    let mut out = [0u8; 8];
    for i in 0..8 {
        let sum = digest[2 * i] as usize + digest[2 * i + 1] as usize;
        out[i] = TBL[sum % 62];
    }
    // TBL 은 ASCII 이므로 항상 유효한 UTF-8 이다.
    String::from_utf8(out.to_vec()).expect("ASCII")
}

#[cfg(test)]
mod tests {
    use super::sofia_hash;

    #[test]
    fn empty_password_matches_known_default() {
        // md5("") = d41d8cd9... → XM 장비의 잘 알려진 기본값
        assert_eq!(sofia_hash(""), "tlJwpbo6");
    }

    #[test]
    fn always_eight_alphanumeric_chars() {
        for pw in ["admin", "1234", "p@ssw0rd!#$", "아주 긴 비밀번호 🙂"] {
            let h = sofia_hash(pw);
            assert_eq!(h.len(), 8, "{pw}");
            assert!(h.bytes().all(|b| b.is_ascii_alphanumeric()), "{pw} -> {h}");
        }
    }

    #[test]
    fn is_deterministic_and_differs_by_password() {
        assert_eq!(sofia_hash("admin"), sofia_hash("admin"));
        assert_ne!(sofia_hash("admin"), sofia_hash("admin1"));
    }
}
