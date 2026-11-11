//! Calculate a natal chart.
//!
//!     FREEASTRO_API_KEY=... cargo run --example natal_chart

use freeastroapi::{Client, params::NatalCalculateParams, types::NatalChartRequest};

#[tokio::main]
async fn main() -> freeastroapi::Result<()> {
    let client = Client::from_env()?;

    let birth = NatalChartRequest {
        year: 1990,
        month: 6,
        day: 15,
        hour: Some(14),
        minute: Some(30),
        city: Some("Paris".into()),
        ..Default::default()
    };
    let options = NatalCalculateParams {
        include_minor: Some(true),
        ..Default::default()
    };
    let chart = client.natal().calculate(&birth, Some(&options)).await?;

    for planet in chart.planets.iter().flatten() {
        println!("{:<10} {} {:.2}°", planet.name, planet.sign, planet.pos);
    }

    let sun = client.western().signs().sun(&birth).await?;
    println!("Sun in {} at {:.2}°", sun.sign, sun.degree);
    Ok(())
}
