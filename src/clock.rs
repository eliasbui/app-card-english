//! The time on the learner's clock, which the reminders, the daily copy and the count in the menu bar follow;
//! std only knows UTC.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Local {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl Local {
    /// The date as the page writes it, `2026-10-27`.
    pub fn date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// Minutes since midnight.
    pub fn minutes(&self) -> u32 {
        self.hour * 60 + self.minute
    }

    /// The seconds since midnight, to find the start of the day from the time now.
    pub fn seconds(&self) -> i64 {
        i64::from(self.minutes() * 60 + self.second)
    }
}

#[cfg(unix)]
pub fn now() -> Option<Local> {
    // SAFETY: `localtime_r` writes only into the `tm` it is given.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return None;
        }
        Some(Local {
            year: tm.tm_year + 1900,
            month: (tm.tm_mon + 1) as u32,
            day: tm.tm_mday as u32,
            hour: tm.tm_hour as u32,
            minute: tm.tm_min as u32,
            second: tm.tm_sec.min(59) as u32,
        })
    }
}

#[cfg(windows)]
pub fn now() -> Option<Local> {
    use windows_sys::Win32::{Foundation::SYSTEMTIME, System::SystemInformation::GetLocalTime};
    // SAFETY: `GetLocalTime` fills the structure it is given and cannot fail.
    let st = unsafe {
        let mut st: SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut st);
        st
    };
    Some(Local {
        year: i32::from(st.wYear),
        month: u32::from(st.wMonth),
        day: u32::from(st.wDay),
        hour: u32::from(st.wHour),
        minute: u32::from(st.wMinute),
        second: u32::from(st.wSecond),
    })
}

#[cfg(not(any(unix, windows)))]
pub fn now() -> Option<Local> {
    None
}

/// `HH:MM` as minutes after midnight.
pub fn parse_hm(text: &str) -> Option<u32> {
    let (h, m) = text.trim().split_once(':')?;
    let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
    (h < 24 && m < 60 && text.trim().len() == 5).then_some(h * 60 + m)
}

/// Whether `minute` falls in the quiet hours from `from` to `to`, which may cross midnight.
pub fn quiet(minute: u32, from: Option<u32>, to: Option<u32>) -> bool {
    match (from, to) {
        (Some(f), Some(t)) if f < t => minute >= f && minute < t,
        (Some(f), Some(t)) if f > t => minute >= f || minute < t,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_and_quiet_hours() {
        assert_eq!(parse_hm("20:30"), Some(20 * 60 + 30));
        assert_eq!(parse_hm("07:05"), Some(425));
        assert_eq!([parse_hm("24:00"), parse_hm("7:05"), parse_hm("12:60"), parse_hm(""), parse_hm("ab:cd")], [None; 5]);
        let (night, morning) = (parse_hm("22:00"), parse_hm("07:00"));
        assert!(quiet(23 * 60, night, morning) && quiet(60, night, morning) && !quiet(7 * 60, night, morning) && !quiet(12 * 60, night, morning));
        assert!(quiet(13 * 60, parse_hm("12:00"), parse_hm("14:00")) && !quiet(14 * 60, parse_hm("12:00"), parse_hm("14:00")));
        assert!(!quiet(0, night, None) && !quiet(0, night, night));
    }

    #[test]
    fn the_clock_reads() {
        let t = now().unwrap();
        assert!(t.year >= 2024 && (1..=12).contains(&t.month) && (1..=31).contains(&t.day) && t.hour < 24 && t.minute < 60);
        assert_eq!(t.date().len(), 10);
        assert!(t.seconds() < 86_400);
    }
}
