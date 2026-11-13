//! The limits, windows and memory use of the check API.

use std::time::Duration;

use refractory::{
    Exceeded, Key, Limiter, ManualClock, MemoryStore, Rule, Store, gcra, normalize_email,
};

const LOGIN: Rule = Rule::new("login", 10, Duration::from_secs(3 * 60));
const LOGIN_PER_ACCOUNT: Rule = Rule::new("login-per-account", 20, Duration::from_secs(3600));
const REGISTRATION: Rule = Rule::new("registration", 5, Duration::from_secs(3600));
const DISCOUNT_CODE: Rule = Rule::new("discount-code", 10, Duration::from_secs(600));
const DELIVERY_ESTIMATE: Rule = Rule::new("delivery-estimate", 20, Duration::from_secs(600));
const INQUIRY: Rule = Rule::new("inquiry", 5, Duration::from_secs(600));
const ORDER: Rule = Rule::new("order", 5, Duration::from_secs(600));
const PASSWORD_RESET: Rule = Rule::new("password-reset", 5, Duration::from_secs(600));

const ALL: [Rule; 8] = [
    LOGIN,
    LOGIN_PER_ACCOUNT,
    REGISTRATION,
    DISCOUNT_CODE,
    DELIVERY_ESTIMATE,
    INQUIRY,
    ORDER,
    PASSWORD_RESET,
];

fn limiter() -> (Limiter<MemoryStore, ManualClock>, ManualClock) {
    let clock = ManualClock::new();
    // Not at zero, so nothing depends on the epoch.
    clock.set(Duration::from_secs(1_000_000));
    (Limiter::builder().clock(clock.clone()).build(), clock)
}

fn allowed_in_a_row(limiter: &Limiter<MemoryStore, ManualClock>, rule: &Rule, key: &str) -> u32 {
    let mut n = 0;
    while limiter.check_sync(rule, key).is_ok() {
        n += 1;
        assert!(n <= rule.limit(), "{rule:?} allowed more than its limit");
    }
    n
}

#[test]
fn a_burst_of_exactly_the_limit_is_allowed_and_one_more_refused() {
    for rule in ALL {
        let (limiter, _) = limiter();
        assert_eq!(
            allowed_in_a_row(&limiter, &rule, "k"),
            rule.limit(),
            "{rule:?}"
        );
        let refused = limiter.check_sync(&rule, "k").unwrap_err();
        assert_eq!(refused.rule, rule.name());
    }
}

#[test]
fn the_shop_limits() {
    let (limiter, _) = limiter();
    let counts: Vec<u32> = ALL
        .iter()
        .map(|rule| allowed_in_a_row(&limiter, rule, "203.0.113.7"))
        .collect();
    assert_eq!(counts, [10, 20, 5, 10, 20, 5, 5, 5]);
}

#[test]
fn refused_events_are_not_counted() {
    let (limiter, clock) = limiter();
    assert_eq!(allowed_in_a_row(&limiter, &INQUIRY, "k"), 5);
    for _ in 0..100 {
        assert!(limiter.check_sync(&INQUIRY, "k").is_err());
    }
    // Hammering did not push the next allowed event further away.
    clock.advance(INQUIRY.interval());
    assert!(limiter.check_sync(&INQUIRY, "k").is_ok());
}

#[test]
fn retry_after_is_the_time_to_the_next_allowed_event() {
    let (limiter, clock) = limiter();
    assert_eq!(allowed_in_a_row(&limiter, &LOGIN, "k"), 10);

    // 3 minutes / 10 = 18 s.
    let refused = limiter.check_sync(&LOGIN, "k").unwrap_err();
    assert_eq!(refused.retry_after, Duration::from_secs(18));
    assert_eq!(refused.retry_after_secs(), 18);

    clock.advance(Duration::from_secs(5));
    let refused = limiter.check_sync(&LOGIN, "k").unwrap_err();
    assert_eq!(refused.retry_after, Duration::from_secs(13));

    clock.advance(Duration::from_millis(12_999));
    let refused = limiter.check_sync(&LOGIN, "k").unwrap_err();
    assert_eq!(refused.retry_after, Duration::from_millis(1));
    assert_eq!(refused.retry_after_secs(), 1);

    clock.advance(Duration::from_millis(1));
    assert!(limiter.check_sync(&LOGIN, "k").is_ok());
    // Exactly one was earned back.
    assert_eq!(
        limiter.check_sync(&LOGIN, "k").unwrap_err().retry_after,
        Duration::from_secs(18)
    );
}

#[test]
fn the_allowance_comes_back_one_interval_at_a_time_and_in_full_after_a_window() {
    let (limiter, clock) = limiter();
    assert_eq!(allowed_in_a_row(&limiter, &REGISTRATION, "k"), 5);

    // One per 12 minutes.
    clock.advance(Duration::from_secs(12 * 60));
    assert_eq!(allowed_in_a_row(&limiter, &REGISTRATION, "k"), 1);
    clock.advance(Duration::from_secs(3 * 12 * 60));
    assert_eq!(allowed_in_a_row(&limiter, &REGISTRATION, "k"), 3);

    // A whole quiet window: the full burst, and no more.
    clock.advance(Duration::from_secs(3600));
    assert_eq!(allowed_in_a_row(&limiter, &REGISTRATION, "k"), 5);
    clock.advance(Duration::from_secs(10 * 3600));
    assert_eq!(allowed_in_a_row(&limiter, &REGISTRATION, "k"), 5);
}

#[test]
fn a_steady_client_gets_the_sustained_rate() {
    let (limiter, clock) = limiter();
    // One delivery estimate every 30 s (20 per 10 minutes) is always fine.
    for _ in 0..1000 {
        assert!(limiter.check_sync(&DELIVERY_ESTIMATE, "k").is_ok());
        clock.advance(Duration::from_secs(30));
    }
    // A little faster than that eventually runs out.
    let mut allowed = 0;
    for _ in 0..1000 {
        if limiter.check_sync(&DELIVERY_ESTIMATE, "k").is_ok() {
            allowed += 1;
        }
        clock.advance(Duration::from_secs(29));
    }
    assert!(allowed < 1000, "{allowed}");
    // ... but not by more than the rate allows over that time, plus a burst.
    assert!(allowed <= 29_000 / 30 + 20, "{allowed}");
}

#[test]
fn no_window_holds_more_than_the_burst_and_the_refill() {
    let (limiter, clock) = limiter();
    let rule = Rule::new("w", 10, Duration::from_secs(100));
    let mut allowed_at = Vec::new();
    // Try once a second for a long time.
    for t in 0..2_000u64 {
        if limiter.check_sync(&rule, "k").is_ok() {
            allowed_at.push(t);
        }
        clock.advance(Duration::from_secs(1));
    }
    for (i, &start) in allowed_at.iter().enumerate() {
        let in_window = allowed_at[i..]
            .iter()
            .take_while(|&&t| t < start + 100)
            .count();
        assert!(in_window < 2 * 10, "{in_window} events from {start}");
    }
    // The long-run rate is the rule's.
    assert!(allowed_at.len() <= 2_000 / 10 + 10, "{}", allowed_at.len());
}

#[test]
fn keys_do_not_interfere() {
    let (limiter, _) = limiter();
    assert_eq!(allowed_in_a_row(&limiter, &INQUIRY, "203.0.113.7"), 5);
    assert_eq!(allowed_in_a_row(&limiter, &INQUIRY, "203.0.113.8"), 5);
    assert!(limiter.check_sync(&INQUIRY, "203.0.113.7").is_err());

    // Keys of other types work too, and equal values are equal keys.
    let ip: std::net::IpAddr = "198.51.100.1".parse().unwrap();
    assert!((0..5).all(|_| limiter.check_sync(&INQUIRY, ip).is_ok()));
    assert!(limiter.check_sync(&INQUIRY, ip).is_err());
    assert!((0..5).all(|_| limiter.check_sync(&INQUIRY, (ip, 7_u32)).is_ok()));
    assert!(
        limiter
            .check_sync(&INQUIRY, String::from("203.0.113.8"))
            .is_err()
    );
}

#[test]
fn rules_do_not_interfere() {
    let (limiter, _) = limiter();
    assert_eq!(allowed_in_a_row(&limiter, &INQUIRY, "k"), 5);
    assert_eq!(allowed_in_a_row(&limiter, &ORDER, "k"), 5);
    assert_eq!(allowed_in_a_row(&limiter, &PASSWORD_RESET, "k"), 5);
    assert!(limiter.check_sync(&INQUIRY, "k").is_err());
}

#[test]
fn login_per_address_and_per_account() {
    let (limiter, clock) = limiter();
    let account = normalize_email(" Jana@Example.cz ");
    let attempt = |ip: &str, email: &str| {
        limiter
            .check_sync(&LOGIN, ip)
            .and_then(|()| limiter.check_sync(&LOGIN_PER_ACCOUNT, normalize_email(email)))
    };

    // One address: 10, then blocked whatever the account.
    for _ in 0..10 {
        assert!(attempt("203.0.113.7", "jana@example.cz").is_ok());
    }
    assert!(attempt("203.0.113.7", "petr@example.cz").is_err());

    // Many addresses against one account: 20 in an hour, typed any way.
    for i in 0..10 {
        assert!(attempt(&format!("198.51.100.{i}"), "JANA@example.cz ").is_ok());
    }
    let refused = attempt("192.0.2.1", &account).unwrap_err();
    assert_eq!(refused.rule, "login-per-account");
    assert_eq!(refused.retry_after, Duration::from_secs(3 * 60));

    // Other accounts from fresh addresses are fine.
    assert!(attempt("192.0.2.1", "petr@example.cz").is_ok());

    clock.advance(Duration::from_secs(3600));
    assert!(attempt("192.0.2.2", &account).is_ok());
}

#[tokio::test]
async fn the_async_check_matches_the_sync_one() {
    let (limiter, _) = limiter();
    for _ in 0..5 {
        assert_eq!(limiter.check(&INQUIRY, "k").await, Ok(()));
    }
    let refused = limiter.check(&INQUIRY, "k").await.unwrap_err();
    assert_eq!(refused.retry_after, Duration::from_secs(120));
    assert!(limiter.check_sync(&INQUIRY, "k").is_err());
    assert_eq!(limiter.try_check(&INQUIRY, "other").await, Ok(Ok(())));
}

#[test]
fn idle_keys_are_cleaned_up() {
    let (limiter, clock) = limiter();
    for i in 0..1000 {
        limiter.check_sync(&INQUIRY, i).unwrap();
    }
    assert_eq!(limiter.store().len(), 1000);

    // Still counting: nothing may be forgotten.
    clock.advance(Duration::from_secs(60));
    limiter.purge();
    assert_eq!(limiter.store().len(), 1000);

    // One event is earned back after 2 minutes; then the key is idle.
    clock.advance(Duration::from_secs(60));
    limiter.purge();
    assert!(limiter.store().is_empty());
}

#[test]
fn the_store_cleans_up_by_itself_as_it_is_used() {
    let (limiter, clock) = limiter();
    for i in 0..10_000 {
        limiter.check_sync(&INQUIRY, i).unwrap();
    }
    assert_eq!(limiter.store().len(), 10_000);

    // Once they are idle, new keys push the old ones out without a purge
    // call, through the size and the time triggers.
    clock.advance(Duration::from_secs(600));
    for i in 0..10_000 {
        limiter.check_sync(&INQUIRY, ("new", i)).unwrap();
        clock.advance(Duration::from_millis(10));
    }
    assert!(limiter.store().len() < 15_000, "{}", limiter.store().len());
}

#[test]
fn a_flood_of_distinct_keys_stays_within_the_cap() {
    let clock = ManualClock::new();
    let limiter = Limiter::builder()
        .clock(clock.clone())
        .store(MemoryStore::new().max_keys(3_200))
        .build();
    // Within one instant, so no key ever becomes idle.
    for i in 0..100_000 {
        limiter.check_sync(&REGISTRATION, i).unwrap();
        assert!(limiter.store().len() <= 3_200);
    }
    assert!(limiter.store().len() >= 2_800, "{}", limiter.store().len());
}

#[test]
fn a_flood_forgets_the_keys_closest_to_idle_first() {
    let clock = ManualClock::new();
    let limiter = Limiter::builder()
        .clock(clock.clone())
        .store(MemoryStore::new().max_keys(320))
        .build();
    // An attacker who used up their allowance just now...
    assert_eq!(
        (0..5)
            .filter(|_| limiter.check_sync(&REGISTRATION, "attacker").is_ok())
            .count(),
        5
    );
    clock.advance(Duration::from_secs(1));
    // ... is not forgotten for one-off keys counted later.
    for i in 0..10_000 {
        limiter.check_sync(&REGISTRATION, i).unwrap();
    }
    assert!(limiter.check_sync(&REGISTRATION, "attacker").is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_checks_allow_exactly_the_limit() {
    let (limiter, _) = limiter();
    let rule = Rule::new("concurrent", 100, Duration::from_secs(3600));
    let mut tasks = Vec::new();
    for task in 0..64 {
        let limiter = limiter.clone();
        tasks.push(tokio::spawn(async move {
            let mut allowed = [0_u32; 4];
            for i in 0..200 {
                let key = (task + i) % 4;
                if limiter.check(&rule, key).await.is_ok() {
                    allowed[key] += 1;
                }
                tokio::task::yield_now().await;
            }
            allowed
        }));
    }
    let mut total = [0_u32; 4];
    for task in tasks {
        for (sum, n) in total.iter_mut().zip(task.await.unwrap()) {
            *sum += n;
        }
    }
    assert_eq!(total, [100; 4]);
}

#[test]
fn concurrent_checks_from_threads() {
    let (limiter, _) = limiter();
    let rule = Rule::new("threads", 1000, Duration::from_secs(3600));
    let allowed: u32 = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    (0..500)
                        .map(|_| u32::from(limiter.check_sync(&rule, "shared").is_ok()))
                        .sum::<u32>()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).sum()
    });
    assert_eq!(allowed, 1000);
}

#[test]
fn keys_are_hashed_with_the_limiter_secret() {
    let a = Limiter::new();
    let b = Limiter::new();
    assert_ne!(
        a.key(&LOGIN, "jana@example.cz"),
        b.key(&LOGIN, "jana@example.cz")
    );
    assert_ne!(a.key(&LOGIN, "k"), a.key(&INQUIRY, "k"));
    assert_eq!(a.key(&LOGIN, "k"), a.key(&LOGIN, String::from("k")));

    let secret = *b"0123456789abcdef";
    let c = Limiter::builder().secret(secret).build();
    let d = Limiter::builder().secret(secret).build();
    assert_eq!(c.key(&LOGIN, "k"), d.key(&LOGIN, "k"));
    assert_eq!(c.key(&LOGIN, "k").to_string().len(), 32);
    assert!(!format!("{:?}", c.store()).contains("example"));
}

/// A store that is always down.
struct Down;

impl Store for Down {
    type Error = &'static str;

    fn check(
        &self,
        _key: Key,
        _rule: Rule,
        _now: Duration,
    ) -> impl Future<Output = Result<Result<(), Exceeded>, Self::Error>> + Send {
        std::future::ready(Err("down"))
    }
}

#[tokio::test]
async fn a_failing_store_fails_open_unless_told_otherwise() {
    let open = Limiter::builder().store(Down).build();
    assert_eq!(open.check(&INQUIRY, "k").await, Ok(()));
    assert_eq!(open.try_check(&INQUIRY, "k").await, Err("down"));

    let closed = Limiter::builder().store(Down).fail_closed(true).build();
    let refused = closed.check(&INQUIRY, "k").await.unwrap_err();
    assert_eq!(refused.retry_after, INQUIRY.interval());
}

#[test]
fn gcra_steps() {
    let rule = Rule::new("g", 3, Duration::from_secs(9));
    let now = Duration::from_secs(50);
    let tat = gcra::step(&rule, None, now).unwrap();
    let tat = gcra::step(&rule, Some(tat), now).unwrap();
    let tat = gcra::step(&rule, Some(tat), now).unwrap();
    assert_eq!(tat, Duration::from_secs(59));
    assert_eq!(
        gcra::step(&rule, Some(tat), now).unwrap_err().retry_after,
        Duration::from_secs(3)
    );
    // A TAT in the past counts as none.
    assert_eq!(
        gcra::step(&rule, Some(Duration::from_secs(1)), now),
        Ok(Duration::from_secs(53))
    );
}

#[test]
fn uneven_windows_still_allow_exactly_the_limit() {
    // 10 ns / 4 does not divide; the burst must still be exactly 4.
    for (limit, nanos) in [
        (4, 10),
        (3, 10),
        (7, 7),
        (1000, 999_999),
        (6, 1_000_000_007),
    ] {
        let (limiter, _) = limiter();
        let rule = Rule::new("uneven", limit, Duration::from_nanos(nanos));
        assert_eq!(
            allowed_in_a_row(&limiter, &rule, "k"),
            limit,
            "{limit} per {nanos} ns"
        );
    }
}

#[test]
#[should_panic(expected = "at least one event")]
fn a_rule_must_allow_something() {
    let _ = Rule::new("never", 0, Duration::from_secs(1));
}
