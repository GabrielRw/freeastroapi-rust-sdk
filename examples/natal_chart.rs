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

    // This response is not typed in the spec yet, so it is a `serde_json::Value`.
    for planet in chart["planets"].as_array().into_iter().flatten() {
        println!("{:<10} {}", planet["name"], planet["sign"]);
    }

    // The sun sign has a typed response.
    let sun = client.western().signs().sun(&birth).await?;
    println!("Sun in {} at {:.2}°", sun.sign, sun.degree);
    Ok(())
}
