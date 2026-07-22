use std::time::{SystemTime, UNIX_EPOCH};

pub fn system_time_to_unix_seconds(time: SystemTime) -> f64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    }
}
