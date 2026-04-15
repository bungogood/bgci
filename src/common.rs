use bkgm::Variant;

pub fn parse_variant(name: &str) -> Result<Variant, String> {
    bkgm::ubgi::parse_variant(name)
}

pub fn variant_name(variant: Variant) -> &'static str {
    bkgm::ubgi::variant_name(variant)
}
