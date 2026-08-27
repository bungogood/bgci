use std::collections::BTreeMap;

use bgci_core::common::Variant;
use bgci_core::config::{EngineLaunch, EngineMetadata, ResolvedEngine, ResolvedMatchup};
use bgci_core::duel_runner::{run_matchup, run_matchup_with_transcripts};

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
        games: 2,
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
    assert_eq!((result.games[0].pair_index, result.games[0].leg), (0, 0));
    assert_eq!((result.games[1].pair_index, result.games[1].leg), (0, 1));
    assert!(
        result
            .games
            .iter()
            .all(|game| [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0].contains(&game.points_a))
    );
    assert!(result.games.iter().all(|game| game.transcript.is_none()));
}

#[tokio::test]
async fn runs_exact_odd_game_counts_with_explicit_singleton_legs() {
    for games in [1, 3] {
        let result = run_matchup(&ResolvedMatchup {
            games,
            parallel: 2,
            seed: 42,
            max_plies: 512,
            variant: Variant::Backgammon,
            engine_a: random_engine("random-a"),
            engine_b: random_engine("random-b"),
        })
        .await
        .unwrap();

        assert_eq!(result.games.len(), games);
        assert_eq!(
            result
                .games
                .iter()
                .map(|game| game.game_idx)
                .collect::<Vec<_>>(),
            (0..games).collect::<Vec<_>>()
        );
        if games == 3 {
            assert_eq!((result.games[0].pair_index, result.games[0].leg), (0, 0));
            assert_eq!((result.games[1].pair_index, result.games[1].leg), (0, 1));
            assert_eq!(result.games[2].pair_index, 1);
        }
    }
}

#[tokio::test]
async fn transcript_capture_is_opt_in_and_preserves_parallel_game_order() {
    let result = run_matchup_with_transcripts(&ResolvedMatchup {
        games: 3,
        parallel: 2,
        seed: 42,
        max_plies: 512,
        variant: Variant::Backgammon,
        engine_a: random_engine("random-a"),
        engine_b: random_engine("random-b"),
    })
    .await
    .unwrap();

    assert_eq!(
        result
            .games
            .iter()
            .map(|game| game.game_idx)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert!(result.games.iter().all(|game| {
        game.transcript
            .as_ref()
            .is_some_and(|turns| turns.len() == game.plies)
    }));
}

#[test]
fn duel_mat_flag_writes_an_ordered_multi_game_session() {
    let path = std::env::temp_dir().join(format!(
        "bgci-mat-{}-{}.mat",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let _ = std::fs::remove_file(&path);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_bgci"))
        .args([
            "duel",
            "--engine-a",
            "random",
            "--engine-b",
            "pipcount",
            "--games",
            "3",
            "--parallel",
            "2",
            "--seed",
            "42",
            "--mat",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let content = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(&path).unwrap();

    assert!(content.starts_with("; [Player 1 \"random\"]"));
    assert_eq!(content.matches(" Game ").count(), 3);
    let game_1 = content.find(" Game 1").unwrap();
    let game_2 = content.find(" Game 2").unwrap();
    let game_3 = content.find(" Game 3").unwrap();
    assert!(game_1 < game_2 && game_2 < game_3);
    assert!(content.contains("0 point match"));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(&format!("mat -> {}", path.display()))
    );
}

#[test]
fn duel_mat_rejects_unsupported_variants_without_creating_the_file() {
    let path =
        std::env::temp_dir().join(format!("bgci-unsupported-mat-{}.mat", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_bgci"))
        .args([
            "duel",
            "--engine-a",
            "random",
            "--engine-b",
            "pipcount",
            "--games",
            "1",
            "--variant",
            "nackgammon",
            "--mat",
        ])
        .arg(&path)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("supports only backgammon"));
    assert!(!path.exists());
}
