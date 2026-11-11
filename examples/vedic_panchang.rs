//! Daily panchang (Vedic calendar) for a place and time.
//!
//!     FREEASTRO_API_KEY=... cargo run --example vedic_panchang

use freeastroapi::{Client, types::PanchangRequestV2};

#[tokio::main]
async fn main() -> freeastroapi::Result<()> {
    let client = Client::from_env()?;

    let panchang = client
        .vedic()
        .panchang(&PanchangRequestV2 {
            year: 2026,
            month: 1,
            day: 14,
            hour: 6,
            minute: 0,
            city: Some("Delhi".into()),
            ..Default::default()
        })
        .await?;

    println!(
        "{} (sunrise {}, sunset {})",
        panchang.date, panchang.sunrise, panchang.sunset
    );
    println!("Weekday:   {}", panchang.weekday.name);
    println!(
        "Tithi:     {} ({} paksha)",
        panchang.tithi.name, panchang.tithi.paksha
    );
    println!("Nakshatra: {}", panchang.nakshatra.name);
    println!("Yoga:      {}", panchang.yoga.name);
    Ok(())
}
