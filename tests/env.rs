//! The API key falls back to FREEASTRO_API_KEY. Kept in its own test binary (own process),
//! because changing the environment races with other threads.

#[test]
fn api_key_falls_back_to_the_environment() {
    // SAFETY: this binary runs a single test, so no other thread reads the environment.
    unsafe { std::env::remove_var(freeastroapi::API_KEY_ENV) };
    assert!(matches!(
        freeastroapi::Client::from_env(),
        Err(freeastroapi::Error::Config(_))
    ));
    // SAFETY: as above.
    unsafe { std::env::set_var(freeastroapi::API_KEY_ENV, "env-key") };
    let client = freeastroapi::Client::from_env().expect("key read from the environment");
    assert_eq!(client.base_url(), freeastroapi::DEFAULT_BASE_URL);
}
