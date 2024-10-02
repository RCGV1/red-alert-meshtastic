use anyhow::Result;
use clap::Parser;
use log::LevelFilter;
use rust_embed::RustEmbed;
use serde::Deserialize;
use simple_logger::SimpleLogger;
use std::collections::HashSet;
use std::process::{Command, Stdio};
use std::time::Duration;
use tokio::time::sleep;
use crate::api::fetch_alert;

mod api;

#[derive(RustEmbed)]
#[folder = "src"]
struct Asset;

#[derive(Debug, Deserialize)]
struct City {
    name: String,
    name_en: String,
    zone_en: String,
}

async fn check_node_connection(args: &Args) -> Result<(), String> {
    // Construct the command to run `meshtastic --info`
    let mut cmd = Command::new("meshtastic");

    // Conditionally add the "--host" argument if the host is provided
    if let Some(host) = &args.host {
        cmd.arg("--host");
        cmd.arg(host);
    }

    // Add the --info argument
    cmd.arg("--info");

    // Ensure the command doesn't output to the console
    cmd.stdout(Stdio::piped());

    // Run the command and capture the output
    let output = cmd.output();

    match output {
        Ok(output) => {
            // Convert the stdout to a string (output is captured as bytes)
            let stdout = String::from_utf8_lossy(&output.stdout);

            // Check if the output contains "Error"
            if stdout.contains("Error") {
                log::error!("Received error output: {}", stdout);
                std::process::exit(1);
            }

            // Check the first line of the output for connection confirmation
            if let Some(first_line) = stdout.lines().next() {
                if first_line == "Connected to radio" {
                    log::info!("Successfully connected to the node.");
                    return Ok(());
                } else {
                    log::error!("Failed to connect to the radio. First line: {}", first_line);
                    std::process::exit(1);
                }
            } else {
                log::error!("Output from meshtastic --info was empty.");
                std::process::exit(1);
            }
        }
        Err(e) => {
            // Log error if the command failed to run
            log::error!("Failed to execute meshtastic --info: {}", e);
            std::process::exit(1);
        }
    }
}


#[derive(Parser, Debug)]
#[command(long_about = None)]
struct Args {
    /// Network address with port of device to connect to in the form of target.address:port
    #[arg(long)]
    host: Option<String>,
}

struct MessageSender {
    last_message_time: Option<std::time::Instant>,
}

impl MessageSender {
    fn new() -> Self {
        MessageSender {
            last_message_time: None,
        }
    }

    async fn send_message_with_retry(
        &mut self,
        chan: u32,
        message: &str,
        retries: u32,
        delay: Duration,
        args: &Args,
    ) -> Result<(), String> {
        if let Some(last_time) = self.last_message_time {
            let elapsed = last_time.elapsed();
            if elapsed < Duration::from_secs(10) {
                sleep(Duration::from_secs(10) - elapsed).await;
            }
        }

        for attempt in 0..=retries {
            let mut command = Command::new("meshtastic");
            command.arg("--ch-index");
            command.arg(chan.to_string());
            command.arg("--sendtext");
            command.arg(message.to_string());

            if let Some(host) = &args.host {
                command.arg("--host").arg(host);
            }

            let result = command.spawn();
            match result {
                Ok(_) => {
                    self.last_message_time = Some(std::time::Instant::now());
                    return Ok(());
                }
                Err(e) => {
                    if attempt < retries {
                        log::warn!("Error sending message: {}. Retrying in {:?}...", e, delay);
                        sleep(delay).await;
                    } else {
                        log::error!("Error sending message after {} attempts: {}", retries, e);
                        return Err(format!("Failed to send message: {}", e));
                    }
                }
            }
        }
        Ok(())
    }
}

// Load Cities.json
async fn load_cities() -> Result<Vec<City>, String> {
    let cities_json = Asset::get("cities.json").ok_or("Failed to load cities.json")?;
    let cities: Vec<City> = serde_json::from_slice(&cities_json.data).map_err(|e| e.to_string())?;
    Ok(cities)
}

// Get the zone number based on zone_en (translated from Hebrew city name)
fn get_zone_number(zone_en: &str) -> Option<u32> {
    // Zone 1: Northern
    if [
        "Upper Galilee",
        "Confrontation Line",
        "North Golan",
        "South Golan",
        "Center Galilee",
    ]
        .contains(&zone_en)
    {
        return Some(1); // Northern
    }

    // Zone 2: NorthCost
    if [
        "HaMifratz",
        "HaCarmel",
        "Menashe",
    ]
        .contains(&zone_en)
    {
        return Some(2); // NorthCoast
    }

    // Zone 3: InterNorth
    if [
        "Lower Galilee",
        "Beit She'an Valley",
        "HaAmakim",
        "Wadi Ara",
    ]
        .contains(&zone_en)
    {
        return Some(3); // InterNorth
    }

    // Zone 4: Central Coast
    if [
        "Sharon",
        "Yarkon",
        "Dan",
    ]
        .contains(&zone_en)
    {
        return Some(4); // Central Coast
    }

    // Zone 5: Central Interior
    if [
        "Shomron",
        "Jerusalem",
        "Yehuda",
        "Shfelat Yehuda",
        "Bika'a",
    ]
        .contains(&zone_en)
    {
        return Some(5); // Central Interior
    }

    // Zone 6: Southern Coast
    if [
        "Gaza Envelope",
        "West Lachish",
        "Lachish",
        "HaShfela",
    ]
        .contains(&zone_en)
    {
        return Some(6); // Southern Coast
    }

    // Zone 7: Desert Region
    if [
        "West Negev",
        "Center Negev",
        "South Negev",
        "Dead Sea",
        "Arava",
        "Eilat",
    ]
        .contains(&zone_en)
    {
        return Some(7); // Desert Region
    }

    None // Return None if the zone_en does not match any known zones
}


// Find zone for a city in Hebrew
async fn find_zone_for_city(cities: &Vec<City>, city_name_he: &str) -> Option<u32> {
    for city in cities {
        if city.name == city_name_he {
            return get_zone_number(&city.zone_en);
        }
    }
    None
}

// Main logic to send alerts to appropriate zones
async fn process_alert(sender: &mut MessageSender, args: &Args, cities: &Vec<City>) -> Result<(), String> {
    // Load city data


    // Fetch the current alert (from the API)
    let alert_result = fetch_alert(false).await.unwrap();

    // Only proceed if there is an actual alert
    if (!alert_result.alert_type.contains("none")) {
        // Check if the alert contains "drill" or "test" (case insensitive)
        if alert_result.alert_type.to_lowercase().contains("drill") || alert_result.alert_type.to_lowercase().contains("test") {
            log::info!("Received a drill or test alert: {}", alert_result.alert_type);
            return Ok(());  // Skip sending the message
        }

        // Prepare a set to store valid zones
        let mut valid_zones = HashSet::new();

        // Find the zones for each city in the alert
        for city in alert_result.cities {
            if let Some(zone) = find_zone_for_city(&cities, &city).await {
                    valid_zones.insert(zone);
            }
        }

        // Create the formatted message based on the reason and instructions
        let message = if let Some(instructions) = &alert_result.instructions {
            format!("🚨{} - {:?}", alert_result.alert_type, instructions)
        } else {
            format!("🚨{}", alert_result.alert_type)
        };
        // Determine which channels to send the alert to
        if valid_zones.len() > 6 {
            // If all zones are valid, send to channel 0
            sender
                .send_message_with_retry(0, &message, 3, Duration::from_secs(5), args)
                .await?;
        } else {
            // Send to each valid zone
            for zone in valid_zones {
                sender
                    .send_message_with_retry(zone, &message, 3, Duration::from_secs(5), args)
                    .await?;
            }
        }
    }

        Ok(())

}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
        // Initialize logging
    SimpleLogger::new()
        .with_level(LevelFilter::Info) // Set to Debug to capture more logs
        .init()
        .unwrap();

    // Parse command-line arguments
    let args = Args::parse();

    let cities = load_cities().await?;

    // Check node connection before starting the loop
    if let Err(e) = check_node_connection(&args).await {
        log::error!("Failed to connect to the node: {}", e);
    } else {
        log::info!("Node connection successful. All systems operational.");
    }

    // Create the message sender
    let mut sender = MessageSender::new();

    // Create an interval to trigger every 5 seconds
    let mut interval = tokio::time::interval(Duration::from_secs(5));

    // Enter the main processing loop
    loop {
        interval.tick().await;

        // Handle process_alert errors without exiting the loop
        if let Err(e) = process_alert(&mut sender, &args, &cities).await {
            log::error!("Error processing alert: {}", e);
        }
    }
}
