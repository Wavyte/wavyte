use super::*;
use crate::animation::ease::Ease;

#[test]
fn wipe_dir_parses_aliases() {
    let spec = TransitionSpec {
        kind: "wipe".to_string(),
        duration_frames: 10,
        ease: Ease::Linear,
        params: serde_json::json!({ "dir": "ttb", "soft_edge": 0.1 }),
    };
    assert_eq!(
        parse_transition(&spec).unwrap(),
        TransitionKind::Wipe {
            dir: WipeDir::TopToBottom,
            soft_edge: 0.1
        }
    );
}

#[test]
fn wipe_soft_edge_is_clamped() {
    let spec = TransitionSpec {
        kind: "wipe".to_string(),
        duration_frames: 10,
        ease: Ease::Linear,
        params: serde_json::json!({ "soft_edge": -5.0 }),
    };
    assert_eq!(
        parse_transition(&spec).unwrap(),
        TransitionKind::Wipe {
            dir: WipeDir::LeftToRight,
            soft_edge: 0.0
        }
    );
}
