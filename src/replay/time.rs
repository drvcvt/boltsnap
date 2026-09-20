pub fn micros(value: i64, numerator: i32, denominator: i32) -> Result<i64, String> {
    if numerator <= 0 || denominator <= 0 {
        return Err("invalid stream time base".into());
    }
    let value = i128::from(value) * i128::from(numerator) * 1_000_000 / i128::from(denominator);
    i64::try_from(value).map_err(|_| "timestamp overflow".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conversion_checks_range_and_time_base() {
        assert_eq!(micros(48_000, 1, 48_000).unwrap(), 1_000_000);
        assert!(micros(1, 1, 0).is_err());
        assert!(micros(i64::MAX, i32::MAX, 1).is_err());
    }
}
