//! Keep absent, null and explicitly present empty strings distinct until Done.
use super::{Budget, ImePreedit, ImePreeditCursor, ImeText, ImeTransaction};
#[derive(Default)]
enum StringPart<T> {
    #[default]
    Absent,
    Null,
    Text(T),
}
/// Data violations poison the batch; protocol and queue failures never reach here.
#[derive(Default)]
pub(super) struct Pending {
    commit: StringPart<ImeText>,
    preedit: StringPart<ImePreedit>,
    deletion: (u32, u32),
    poisoned: Option<&'static str>,
}
impl Pending {
    pub(super) fn commit(&mut self, text: Option<String>, budget: &Budget) {
        if self.poisoned.is_some() {
            return;
        }
        match text {
            Some(text) => match budget.text(text) {
                Ok(text) => self.commit = StringPart::Text(ImeText(text)),
                Err(reason) => self.poison(reason),
            },
            None => self.commit = StringPart::Null,
        }
    }
    pub(super) fn preedit(&mut self, text: Option<String>, begin: i32, end: i32, budget: &Budget) {
        if self.poisoned.is_some() {
            return;
        }
        let part = match text {
            Some(text) => {
                let cursor = match cursor(&text, begin, end) {
                    Ok(cursor) => cursor,
                    Err(reason) => return self.poison(reason),
                };
                match budget.text(text) {
                    Ok(text) => StringPart::Text(ImePreedit {
                        text: ImeText(text),
                        cursor,
                    }),
                    Err(reason) => return self.poison(reason),
                }
            }
            None if (begin, end) == (-1, -1) || (begin, end) == (0, 0) => StringPart::Null,
            None => return self.poison("cursor on null IME preedit"),
        };
        self.preedit = part;
    }
    pub(super) fn delete(&mut self, before: u32, after: u32) {
        if self.poisoned.is_none() {
            self.deletion = (before, after);
        }
    }
    /// Drops every accepted payload at once so its ledger credits return.
    fn poison(&mut self, reason: &'static str) {
        *self = Self {
            poisoned: Some(reason),
            ..Self::default()
        };
    }
    pub(super) fn poisoned(&self) -> Option<&'static str> {
        self.poisoned
    }
    pub(super) fn take(&mut self) -> ImeTransaction {
        let pending = std::mem::take(self);
        ImeTransaction {
            delete_before: pending.deletion.0,
            delete_after: pending.deletion.1,
            commit: match pending.commit {
                StringPart::Text(text) => Some(text),
                _ => None,
            },
            preedit: match pending.preedit {
                StringPart::Text(text) => Some(text),
                _ => None,
            },
        }
    }
}
fn cursor(text: &str, begin: i32, end: i32) -> Result<Option<ImePreeditCursor>, &'static str> {
    if (begin, end) == (-1, -1) {
        return Ok(None);
    }
    let begin = usize::try_from(begin).map_err(|_| "negative IME cursor")?;
    let end = usize::try_from(end).map_err(|_| "negative IME cursor")?;
    if !text.is_char_boundary(begin) || !text.is_char_boundary(end) {
        return Err("IME cursor is not a scalar boundary");
    }
    Ok(Some(ImePreeditCursor { begin, end }))
}

#[cfg(test)]
#[path = "tests/pending.rs"]
mod tests;
