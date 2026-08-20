use std::collections::BTreeMap;

use bgci_core::common::Variant;
use bgci_core::config::{EngineLaunch, EngineMetadata, ResolvedEngine, ResolvedMatchup};
use bgci_core::duel_runner::run_matchup;

fn random_engine(name: &str) -> ResolvedEngine {
    ResolvedEngine {
        name: name.to_string(),
        launch: EngineLaunch::new(
            vec![
                env!("CARGO_BIN_EXE_bgci").to_string(),
                "engine".to_string(),
                "random".to_string(),
            ],
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap(),
        metadata: EngineMetadata::default(),
    }
}

#[tokio::test]
async fn runs_a_mirrored_pair_through_real_ubgi_processes() {
    let result = run_matchup(&ResolvedMatchup {
        pairs: 1,
        parallel: 1,
        seed: 42,
        max_plies: 512,
        variant: Variant::Backgammon,
        engine_a: random_engine("random-a"),
        engine_b: random_engine("random-b"),
    })
    .await
    .unwrap();

    assert_eq!(result.games.len(), 2);
    assert_eq!(result.games[0].game_idx, 0);
    assert_eq!(result.games[1].game_idx, 1);
    assert!(
        result
            .games
            .iter()
            .all(|game| [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0].contains(&game.points_a))
    );
}
