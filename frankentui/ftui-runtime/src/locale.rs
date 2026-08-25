#![forbid(unsafe_code)]

//! Locale context provider for runtime-wide internationalization.
//!
//! The [`LocaleContext`] owns the current locale and exposes scoped overrides
//! for widget subtrees. Locale changes are versioned so the runtime can
//! trigger re-renders when the active locale changes.

use crate::reactive::{Observable, Subscription};
pub use ftui_i18n::catalog::Locale;
use std::cell::RefCell;
use std::env;
use std::rc::Rc;

thread_local! {
    static GLOBAL_CONTEXT: LocaleContext = LocaleContext::system();
}

/// Runtime locale context with scoped overrides.
#[derive(Clone, Debug)]
pub struct LocaleContext {
    current: Observable<Locale>,
    overrides: Rc<RefCell<Vec<Locale>>>,
}

impl LocaleContext {
    /// Create a new locale context with the provided locale.
    #[must_use]
    pub fn new(locale: impl Into<Locale>) -> Self {
        let locale = normalize_locale(locale.into());
        Self {
            current: Observable::new(locale),
            overrides: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Create a locale context initialized from system locale detection.
    #[must_use]
    pub fn system() -> Self {
        Self::new(detect_system_locale())
    }

    /// Access the global locale context (thread-local).
    #[must_use]
    pub fn global() -> Self {
        GLOBAL_CONTEXT.with(Clone::clone)
    }

    /// Get the active locale, honoring any scoped override.
    #[must_use]
    pub fn current_locale(&self) -> Locale {
        if let Some(locale) = self.overrides.borrow().last() {
            locale.clone()
        } else {
            self.current.get()
        }
    }

    /// Get the base locale without considering overrides.
    #[must_use]
    pub fn base_locale(&self) -> Locale {
        self.current.get()
    }

    /// Set the base locale.
    pub fn set_locale(&self, locale: impl Into<Locale>) {
        let locale = normalize_locale(locale.into());
        self.current.set(locale);
    }

    /// Subscribe to base locale changes.
    pub fn subscribe(&self, callback: impl Fn(&Locale) + 'static) -> Subscription {
        self.current.subscribe(callback)
    }

    /// Push a scoped locale override. Dropping the guard restores the prior locale.
    #[must_use = "dropping this guard clears the locale override"]
    pub fn push_override(&self, locale: impl Into<Locale>) -> LocaleOverride {
        let locale = normalize_locale(locale.into());
        self.overrides.borrow_mut().push(locale.clone());
        LocaleOverride {
            stack: Rc::clone(&self.overrides),
            locale,
        }
    }

    /// Current version counter for the base locale.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.current.version()
    }
}

/// RAII guard for scoped locale overrides.
#[must_use = "dropping this guard clears the locale override"]
pub struct LocaleOverride {
    stack: Rc<RefCell<Vec<Locale>>>,
    locale: Locale,
}

impl Drop for LocaleOverride {
    fn drop(&mut self) {
        let popped = self.stack.borrow_mut().pop();
        if let Some(popped) = popped {
            debug_assert_eq!(popped, self.locale);
        }
    }
}

/// Detect the system locale from environment variables.
///
/// Preference order: `LC_ALL`, then `LANG`. Falls back to `"en"` when unknown.
#[must_use]
pub fn detect_system_locale() -> Locale {
    let lc_all = env::var("LC_ALL").ok();
    let lang = env::var("LANG").ok();
    detect_system_locale_from(lc_all.as_deref(), lang.as_deref())
}

/// Convenience: set the global locale.
pub fn set_locale(locale: impl Into<Locale>) {
    LocaleContext::global().set_locale(locale);
}

/// Convenience: get the global locale.
#[must_use]
pub fn current_locale() -> Locale {
    LocaleContext::global().current_locale()
}

fn normalize_locale(mut locale: Locale) -> Locale {
    normalize_locale_raw(&locale).unwrap_or_else(|| {
        locale.clear();
        locale.push_str("en");
        locale
    })
}

fn detect_system_locale_from(lc_all: Option<&str>, lang: Option<&str>) -> Locale {
    lc_all
        .and_then(normalize_locale_raw)
        .or_else(|| lang.and_then(normalize_locale_raw))
        .unwrap_or_else(|| "en".to_string())
}

fn normalize_locale_raw(raw: &str) -> Option<Locale> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let raw = raw.split('@').next().unwrap_or(raw);
    let raw = raw.split('.').next().unwrap_or(raw);
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let mut normalized = raw.replace('_', "-");
    if normalized.eq_ignore_ascii_case("c") || normalized.eq_ignore_ascii_case("posix") {
        normalized.clear();
        normalized.push_str("en");
    }
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // ---------------------------------------------------------------------
    // Invariants (Alien Artifact)
    // ---------------------------------------------------------------------
    // 1. Normalized locales contain no '_' '.' or '@' suffixes.
    // 2. Locale overrides are LIFO and never mutate the base locale.
    // 3. Locale versions only advance on base locale changes.
    //
    // Failure Modes:
    // | Scenario                     | Expected Behavior                 |
    // |-----------------------------|-----------------------------------|
    // | Empty / whitespace locale   | Falls back to "en"                |
    // | "C"/"POSIX" locale          | Normalized to "en"                |
    // | Override drop out of order  | Debug assert (in dev builds)      |

    #[test]
    fn detect_system_locale_prefers_lc_all() {
        let locale = detect_system_locale_from(Some("fr_FR.UTF-8"), Some("en_US.UTF-8"));
        assert_eq!(locale, "fr-FR");
    }

    #[test]
    fn detect_system_locale_uses_lang_when_lc_all_missing() {
        let locale = detect_system_locale_from(None, Some("en_US.UTF-8"));
        assert_eq!(locale, "en-US");
    }

    #[test]
    fn detect_system_locale_defaults_to_en() {
        let locale = detect_system_locale_from(None, None);
        assert_eq!(locale, "en");
    }

    #[test]
    fn locale_context_switching_updates_version() {
        let ctx = LocaleContext::new("en");
        let v0 = ctx.version();
        ctx.set_locale("en");
        assert_eq!(ctx.version(), v0);
        ctx.set_locale("es");
        assert!(ctx.version() > v0);
        assert_eq!(ctx.current_locale(), "es");
    }

    #[test]
    fn locale_override_is_scoped() {
        let ctx = LocaleContext::new("en");
        assert_eq!(ctx.current_locale(), "en");
        let guard = ctx.push_override("fr");
        assert_eq!(ctx.current_locale(), "fr");
        drop(guard);
        assert_eq!(ctx.current_locale(), "en");
    }

    #[test]
    fn locale_override_is_lifo() {
        let ctx = LocaleContext::new("en");
        let _outer = ctx.push_override("fr");
        assert_eq!(ctx.current_locale(), "fr");
        {
            let _inner = ctx.push_override("es");
            assert_eq!(ctx.current_locale(), "es");
        }
        assert_eq!(ctx.current_locale(), "fr");
    }

    #[test]
    fn normalize_locale_handles_c_and_posix() {
        let c_locale = normalize_locale_raw("C");
        let posix_locale = normalize_locale_raw("POSIX");
        assert_eq!(c_locale.as_deref(), Some("en"));
        assert_eq!(posix_locale.as_deref(), Some("en"));
    }

    #[test]
    fn normalize_locale_strips_codeset_and_modifier() {
        let locale = normalize_locale_raw("en_US.UTF-8@latin");
        assert_eq!(locale.as_deref(), Some("en-US"));
    }

    #[test]
    fn locale_override_does_not_mutate_base_locale() {
        let ctx = LocaleContext::new("en");
        let v0 = ctx.version();
        let _guard = ctx.push_override("fr");
        assert_eq!(ctx.base_locale(), "en");
        assert_eq!(ctx.version(), v0);
    }

    #[test]
    fn normalize_empty_falls_back_to_en() {
        let locale = normalize_locale("".to_string());
        assert_eq!(locale, "en");
    }

    #[test]
    fn normalize_whitespace_only_falls_back_to_en() {
        let locale = normalize_locale("   ".to_string());
        assert_eq!(locale, "en");
    }

    #[test]
    fn subscribe_fires_on_change() {
        use std::cell::Cell;
        use std::rc::Rc;

        let ctx = LocaleContext::new("en");
        let fired = Rc::new(Cell::new(false));
        let fired_clone = Rc::clone(&fired);
        let _sub = ctx.subscribe(move |_| {
            fired_clone.set(true);
        });
        ctx.set_locale("de");
        assert!(fired.get());
    }

    #[test]
    fn detect_system_locale_empty_lc_all_uses_lang() {
        let locale = detect_system_locale_from(Some(""), Some("ja_JP.UTF-8"));
        assert_eq!(locale, "ja-JP");
    }

    proptest! {
        #[test]
        fn normalize_locale_raw_sanitizes_segments(raw in "[A-Za-z0-9_@.\\-]{1,32}") {
            let normalized = normalize_locale_raw(&raw);
            if let Some(locale) = normalized {
                prop_assert!(!locale.trim().is_empty());
                prop_assert!(!locale.contains('@'));
                prop_assert!(!locale.contains('.'));
                prop_assert!(!locale.contains('_'));
            }
        }

        #[test]
        fn overrides_are_lifo(locales in proptest::collection::vec("[a-z]{2}(-[A-Z]{2})?", 1..6)) {
            let ctx = LocaleContext::new("en");
            let mut guards = Vec::new();
            for locale in &locales {
                guards.push(ctx.push_override(locale));
            }
            prop_assert_eq!(ctx.current_locale(), locales.last().unwrap().as_str());
            guards.pop();
            if locales.len() >= 2 {
                let prev = &locales[locales.len() - 2];
                prop_assert_eq!(ctx.current_locale(), prev.as_str());
            } else {
                prop_assert_eq!(ctx.current_locale(), "en");
            }
            // Drop remaining guards in LIFO order (Vec drops front-to-back,
            // but the override stack expects last-pushed-first-dropped).
            while guards.pop().is_some() {}
        }
    }
}
