//! Validation logic for label and metric names.

const fn is_valid_start_name_char(ch: u8) -> bool {
    ch == b'_' || ch.is_ascii_lowercase()
}

const fn is_valid_name_char(ch: u8) -> bool {
    ch == b'_' || ch.is_ascii_lowercase() || ch.is_ascii_digit()
}

const fn validate_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("name cannot be empty");
    }

    let name_bytes = name.as_bytes();
    let mut idx = 0;
    while idx < name.len() {
        if name_bytes[idx] > 127 {
            return Err("name contains non-ASCII chars");
        }
        if idx == 0 && !is_valid_start_name_char(name_bytes[idx]) {
            return Err("name starts with disallowed char (allowed chars: [_a-z])");
        } else if !is_valid_name_char(name_bytes[idx]) {
            return Err("name contains disallowed char (allowed chars: [_a-z0-9])");
        }
        idx += 1;
    }
    Ok(())
}

/// Checks that a label name is valid.
#[track_caller]
pub const fn assert_label_name(name: &str) {
    if let Err(err) = validate_name(name) {
        panic!("{}", err);
    }
}

/// Same as [`assert_label_name()`], but for multiple names.
#[track_caller]
pub const fn assert_label_names(names: &[&str]) {
    let mut idx = 0;
    while idx < names.len() {
        assert_label_name(names[idx]);
        idx += 1;
    }
}

/// Checks that a metric name is valid.
#[track_caller]
pub const fn assert_metric_name(name: &str) {
    if let Err(err) = validate_name(name) {
        panic!("{}", err);
    }
}

/// Checks that a metric prefix is valid.
#[track_caller]
pub const fn assert_metric_prefix(name: &str) {
    if let Err(err) = validate_name(name) {
        panic!("{}", err);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validating_names() {
        let valid_names = ["test", "_private", "snake_case", "l33t_c0d3"];
        for name in valid_names {
            validate_name(name).unwrap();
        }

        validate_name("").unwrap_err();
        validate_name("нет").unwrap_err();
        validate_name("t!st").unwrap_err();
        validate_name("1est").unwrap_err();
    }
}
