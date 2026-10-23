//! The scheduler counts of a node: the default of the host and the `+S`
//! emulator flag, as the pinned `erl_init.c` sets them.

/// The most schedulers of a node (`ERTS_MAX_NO_OF_SCHEDULERS`,
/// `erl_process.h:82`).
const MAX_SCHEDULERS: i32 = 1024;

/// The number of schedulers and the number of schedulers online of a node.
///
/// ERTS starts one scheduler for each configured logical processor, and
/// sets one scheduler online for each processor that the process can use
/// (`erl_init.c:848-860`). PBeam reads one count, the processors that the
/// process can use (`std::thread::available_parallelism`), for both (row
/// "Scheduling" of `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// let mut schedulers = Schedulers::host();
/// schedulers.apply_flag(b"4:2")?;
/// assert_eq!((schedulers.total(), schedulers.online()), (4, 2));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedulers {
    /// The default count of schedulers (`no_schedulers` before the flags).
    host_total: i32,
    /// The default count of schedulers online.
    host_online: i32,
    /// The count of schedulers (`schdlrs`).
    total: i32,
    /// The count of schedulers online (`schdlrs_onln`).
    online: i32,
}

impl Schedulers {
    /// Return the default counts of the host.
    pub fn host() -> Self {
        let count = std::thread::available_parallelism().map_or(1, |count| {
            i32::try_from(count.get()).unwrap_or(MAX_SCHEDULERS)
        });
        Schedulers {
            host_total: count,
            host_online: count,
            total: count,
            online: count,
        }
    }

    /// Return the count of schedulers, which `erlang:system_info(schedulers)`
    /// gives.
    pub fn total(&self) -> usize {
        usize::try_from(self.total).expect("a checked count is positive")
    }

    /// Return the count of schedulers online, which
    /// `erlang:system_info(schedulers_online)` gives. The node starts one
    /// scheduler thread for each.
    pub fn online(&self) -> usize {
        usize::try_from(self.online).expect("a checked count is positive")
    }

    /// Apply the value `argument` of a `+S` flag, as `erl_init.c:1103-1149`
    /// does: `Total:Online`, `Total`, or `:Online`. A value of 0 or less
    /// counts down from the default of the host. A later flag starts from
    /// the counts of the earlier flags.
    ///
    /// # Errors
    ///
    /// The text that ERTS writes before its usage text: `bad amount of
    /// schedulers <Argument>` for a value that does not parse, `bad amount
    /// of schedulers <Total>` for a total outside 1 to 1024, and `bad amount
    /// of schedulers online <Online> (total amount of schedulers <Total>)`
    /// for an online count outside 1 to the total.
    pub fn apply_flag(&mut self, argument: &[u8]) -> Result<(), String> {
        let (total, online) = match scan_int(argument) {
            // `sscanf(arg, "%d:%d")` gives 2, or 1 with the default online
            // count (`erl_init.c:1115-1117`).
            Some((total, rest)) => match rest.strip_prefix(b":").and_then(scan_int) {
                Some((online, _)) => (total, online),
                None => (total, total.min(self.online)),
            },
            // `sscanf(arg, ":%d")` (`erl_init.c:1107-1114`).
            None => match argument.strip_prefix(b":").and_then(scan_int) {
                Some((online, _)) => (self.host_total, online),
                None => {
                    return Err(format!(
                        "bad amount of schedulers {}",
                        String::from_utf8_lossy(argument)
                    ));
                }
            },
        };
        let count = if total > 0 {
            total
        } else {
            self.host_total.wrapping_add(total)
        };
        let count_online = if online > 0 {
            online
        } else {
            self.host_online.wrapping_add(online)
        };
        if !(1..=MAX_SCHEDULERS).contains(&count) {
            return Err(format!("bad amount of schedulers {total}"));
        }
        if count_online < 1 || count < count_online {
            return Err(format!(
                "bad amount of schedulers online {count_online} (total amount of schedulers {count})"
            ));
        }
        self.total = count;
        self.online = count_online;
        Ok(())
    }
}

/// Scan one `%d` conversion of `sscanf` at the start of `text`: white space,
/// an optional sign, and at least one digit. Return the value and the rest
/// of `text`. The C library reads the digits as a `long`, which saturates,
/// and stores the `long` in an `int`, which truncates it.
fn scan_int(text: &[u8]) -> Option<(i32, &[u8])> {
    let text = text.trim_ascii_start();
    let (negative, digits) = match text.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        _ => (false, text),
    };
    let end = digits
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    let value = digits[..end].iter().fold(0i64, |value, digit| {
        let digit = i64::from(digit - b'0');
        if negative {
            value.saturating_mul(10).saturating_sub(digit)
        } else {
            value.saturating_mul(10).saturating_add(digit)
        }
    });
    Some((value as i32, &digits[end..]))
}

#[cfg(test)]
#[path = "../../tests/runtime/schedulers.rs"]
mod tests;
