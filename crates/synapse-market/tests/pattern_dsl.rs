use synapse_market::Pattern;
use synapse_market::parse_pattern;

#[test]
fn test_dsl_drought_buy() {
    let p = parse_pattern("DroughtBuy(quiet=180, min=250000)").unwrap();
    assert!(matches!(
        p,
        Pattern::DroughtBuy {
            quiet_days: 180,
            min_value: 250000.0
        }
    ));
}

#[test]
fn test_dsl_insider_cluster() {
    let p = parse_pattern("InsiderCluster(k=3, w=30)").unwrap();
    assert!(matches!(
        p,
        Pattern::InsiderCluster {
            k: 3,
            window_days: 30
        }
    ));
}

#[test]
fn test_dsl_fda_triple() {
    let p = parse_pattern("FdaTriple(w=60)").unwrap();
    assert!(matches!(p, Pattern::FdaTriple { window_days: 60 }));
}

#[test]
fn test_dsl_volume_spike() {
    let p = parse_pattern("VolumeSpike(mult=2.5, w=20)").unwrap();
    assert!(matches!(
        p,
        Pattern::VolumeSpike {
            multiplier: _,
            window_bars: 20
        }
    ));
    if let Pattern::VolumeSpike { multiplier, .. } = p {
        assert!((multiplier - 2.5).abs() < 0.001);
    }
}

#[test]
fn test_dsl_then() {
    let p = parse_pattern("InsiderCluster(k=3, w=30) THEN VolumeSpike(mult=2.5, w=20) within 14")
        .unwrap();
    assert!(matches!(p, Pattern::Then(_, _, 14)));
}

#[test]
fn test_dsl_unknown_pattern() {
    assert!(parse_pattern("Unknown(x=1)").is_err());
}
