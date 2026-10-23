//! Text formatting for durations, dates and languages (exports, titles).

use chrono::{DateTime, Datelike, Local, Timelike};

use crate::model::Language;

/// "2:34" or "1:02:03" (rounded down).
pub fn duration(seconds: f64) -> String {
    let total = seconds.max(0.0).floor() as u64;
    let (hours, minutes, secs) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

const MONTHS_DE: [&str; 12] = [
    "Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember",
];
const MONTHS_EN: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
];

/// "23. Oktober 2026, 14:05" / "October 23, 2026 at 2:05 PM".
pub fn long_date_time(date: DateTime<Local>) -> String {
    let month = date.month0() as usize;
    match Language::current() {
        Language::German => {
            format!("{}. {} {}, {:02}:{:02}", date.day(), MONTHS_DE[month], date.year(), date.hour(), date.minute())
        }
        Language::English => {
            let (pm, hour) = date.hour12();
            format!(
                "{} {}, {} at {}:{:02} {}",
                MONTHS_EN[month],
                date.day(),
                date.year(),
                hour,
                date.minute(),
                if pm { "PM" } else { "AM" }
            )
        }
    }
}

/// "23.10.2026, 14:05" / "10/23/2026, 2:05 PM" — for default titles.
pub fn short_date_time(date: DateTime<Local>) -> String {
    match Language::current() {
        Language::German => {
            format!("{:02}.{:02}.{}, {:02}:{:02}", date.day(), date.month(), date.year(), date.hour(), date.minute())
        }
        Language::English => {
            let (pm, hour) = date.hour12();
            format!(
                "{}/{}/{}, {}:{:02} {}",
                date.month(),
                date.day(),
                date.year(),
                hour,
                date.minute(),
                if pm { "PM" } else { "AM" }
            )
        }
    }
}

/// Language name for a BCP-47 code, e.g. "de" → "Deutsch".
pub fn language_name(code: &str) -> String {
    let german = Language::current() == Language::German;
    let name = match code {
        "de" => if german { "Deutsch" } else { "German" },
        "en" => if german { "Englisch" } else { "English" },
        "fr" => if german { "Französisch" } else { "French" },
        "es" => if german { "Spanisch" } else { "Spanish" },
        "it" => if german { "Italienisch" } else { "Italian" },
        "nl" => if german { "Niederländisch" } else { "Dutch" },
        "pt" => if german { "Portugiesisch" } else { "Portuguese" },
        "pl" => if german { "Polnisch" } else { "Polish" },
        "cs" => if german { "Tschechisch" } else { "Czech" },
        "sk" => if german { "Slowakisch" } else { "Slovak" },
        "sv" => if german { "Schwedisch" } else { "Swedish" },
        "da" => if german { "Dänisch" } else { "Danish" },
        "nb" => if german { "Norwegisch" } else { "Norwegian" },
        "fi" => if german { "Finnisch" } else { "Finnish" },
        "hu" => if german { "Ungarisch" } else { "Hungarian" },
        "ro" => if german { "Rumänisch" } else { "Romanian" },
        "bg" => if german { "Bulgarisch" } else { "Bulgarian" },
        "hr" => if german { "Kroatisch" } else { "Croatian" },
        "sl" => if german { "Slowenisch" } else { "Slovenian" },
        "et" => if german { "Estnisch" } else { "Estonian" },
        "lv" => if german { "Lettisch" } else { "Latvian" },
        "lt" => if german { "Litauisch" } else { "Lithuanian" },
        "el" => if german { "Griechisch" } else { "Greek" },
        "uk" => if german { "Ukrainisch" } else { "Ukrainian" },
        "ru" => if german { "Russisch" } else { "Russian" },
        other => return other.to_string(),
    };
    name.to_string()
}

/// A title that is safe as a Windows file name.
pub fn file_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() { '-' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    if cleaned.is_empty() {
        match Language::current() {
            Language::German => "Transkript".into(),
            Language::English => "Transcript".into(),
        }
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration(0.0), "0:00");
        assert_eq!(duration(1.2), "0:01");
        assert_eq!(duration(154.9), "2:34");
        assert_eq!(duration(3_661.5), "1:01:01");
        assert_eq!(duration(3_725.0), "1:02:05");
    }

    #[test]
    fn file_names() {
        assert_eq!(file_name("Meeting 18.10.2026, 11:57"), "Meeting 18.10.2026, 11-57");
        assert_eq!(file_name("a/b\\c"), "a-b-c");
        assert!(!file_name("   ").is_empty());
    }
}
