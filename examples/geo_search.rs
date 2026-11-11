//! Look up a city's coordinates and timezone, with typed error handling.
//!
//!     FREEASTRO_API_KEY=... cargo run --example geo_search -- "San Francisco"

use freeastroapi::{Client, Error, params::GeoSearchParams};

#[tokio::main]
async fn main() {
    let query = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Paris".to_owned());
    let client = match Client::from_env() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    let params = GeoSearchParams {
        q: query,
        limit: Some(5),
        ..Default::default()
    };
    match client.geo().search(&params).await {
        Ok(cities) => {
            for city in cities.results {
                println!(
                    "{}, {} ({:.4}, {:.4}) {}",
                    city.name, city.country, city.lat, city.lng, city.timezone
                );
            }
        }
        Err(Error::Authentication(error)) => eprintln!("check FREEASTRO_API_KEY: {error}"),
        Err(Error::RateLimit { retry_after, .. }) => {
            eprintln!("rate limited; retry after {retry_after:?}")
        }
        Err(Error::Validation(error)) => eprintln!("invalid query: {}", error.message),
        Err(error) => eprintln!("request failed: {error}"),
    }
}
