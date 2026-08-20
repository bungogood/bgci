pub use bkgm::Variant;

pub fn parse_variant(name: &str) -> Result<Variant, String> {
    match name.trim().to_ascii_lowercase().as_str() {
        "backgammon" | "bg" => Ok(Variant::Backgammon),
        "nackgammon" | "nack" => Ok(Variant::Nackgammon),
        "longgammon" | "long" => Ok(Variant::Longgammon),
        "hypergammon" | "hyper" | "hypergammon3" => Ok(Variant::Hypergammon),
        "hypergammon2" | "hyper2" => Ok(Variant::Hypergammon2),
        "hypergammon4" | "hyper4" => Ok(Variant::Hypergammon4),
        "hypergammon5" | "hyper5" => Ok(Variant::Hypergammon5),
        _ => Err(format!("unknown variant: {name}")),
    }
}

pub fn variant_name(variant: Variant) -> &'static str {
    match variant {
        Variant::Backgammon => "backgammon",
        Variant::Nackgammon => "nackgammon",
        Variant::Longgammon => "longgammon",
        Variant::Hypergammon => "hypergammon",
        Variant::Hypergammon2 => "hypergammon2",
        Variant::Hypergammon4 => "hypergammon4",
        Variant::Hypergammon5 => "hypergammon5",
    }
}
