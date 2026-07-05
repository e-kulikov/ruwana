use rand::Rng;

const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

fn generate(len: usize) -> String {
    // rand's ThreadRng is a CSPRNG (spec: ID Generation).
    let mut rng = rand::rng();
    (0..len).map(|_| CHARSET[rng.random_range(0..CHARSET.len())] as char).collect()
}

pub fn generate_task_id() -> String {
    generate(8)
}

pub fn generate_subtask_id() -> String {
    generate(4)
}

pub fn is_task_id_shaped(s: &str) -> bool {
    s.len() == 8 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_charset(s: &str) {
        assert!(s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()), "bad charset: {s}");
    }

    #[test]
    fn task_ids_are_8_lowercase_alnum() {
        for _ in 0..100 {
            let id = generate_task_id();
            assert_eq!(id.len(), 8);
            assert_charset(&id);
        }
    }

    #[test]
    fn subtask_ids_are_4_lowercase_alnum() {
        for _ in 0..100 {
            let id = generate_subtask_id();
            assert_eq!(id.len(), 4);
            assert_charset(&id);
        }
    }

    #[test]
    fn generated_ids_vary() {
        let ids: std::collections::HashSet<String> = (0..50).map(|_| generate_task_id()).collect();
        assert!(ids.len() > 1, "RNG appears constant");
    }

    #[test]
    fn id_shape_check() {
        assert!(is_task_id_shaped("a3bc9f2e"));
        assert!(is_task_id_shaped("00000000"));
        assert!(!is_task_id_shaped("a3bc9f2"));      // 7 chars
        assert!(!is_task_id_shaped("a3bc9f2ef"));    // 9 chars
        assert!(!is_task_id_shaped("A3BC9F2E"));     // uppercase
        assert!(!is_task_id_shaped("a3bc-f2e"));     // punctuation
        assert!(!is_task_id_shaped("Fix tests"));    // a title
    }
}
