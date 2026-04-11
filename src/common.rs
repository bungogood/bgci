use bkgm::Variant;

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

pub fn parse_variant_setoption(cmd: &str) -> Option<Result<Variant, String>> {
    let mut parts = cmd.split_whitespace();
    let setoption = parts.next()?;
    let name_kw = parts.next()?;
    let option_name = parts.next()?;
    let value_kw = parts.next()?;
    let value = parts.next()?;

    if !setoption.eq_ignore_ascii_case("setoption")
        || !name_kw.eq_ignore_ascii_case("name")
        || !value_kw.eq_ignore_ascii_case("value")
        || !option_name.eq_ignore_ascii_case("variant")
        || parts.next().is_some()
    {
        return None;
    }

    Some(parse_variant(value))
}
