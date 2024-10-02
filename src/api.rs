use std::error::Error;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use serde_json::{json, Value};
use tokio::time::Duration;

const CONFIG_API: &str = "https://www.oref.org.il/WarningMessages/alert/alerts.json";
const CONFIG_HISTORY_API: &str = "https://www.oref.org.il/WarningMessages/alert/alertsHistory.json";

// Alert type structure
#[derive(Debug, Deserialize, Serialize)]
struct Alert {
    #[serde(rename = "data")]
    cities: Option<Vec<String>>,
    #[serde(rename = "cat")]
    category: Option<String>,
    #[serde(rename = "desc")]
    instructions: Option<String>,
}

// History alert structure
#[derive(Debug, Deserialize)]
struct HistoryAlert {
    alertDate: Option<String>,
    data: Option<String>,
    category: Option<String>,
}

#[derive(Debug)]
pub struct AlertResult {
    pub alert_type: String,
    pub cities: Vec<String>,
    pub instructions: Option<String>,
}

// Main async function to fetch and extract the alert
pub async fn fetch_alert(alert_history: bool) -> Result<AlertResult, Box<dyn std::error::Error>> {
    let json = get_hfc_alerts_json(alert_history).await?;
    let alert = extract_alert_from_json(json).await?;
    Ok(alert)
}

// Async function to perform the HTTP request to HFC API
async fn get_hfc_alerts_json(alert_history: bool) -> Result<Value, Box<dyn Error>> {
    let api_url = if alert_history { CONFIG_HISTORY_API } else { CONFIG_API };

    let unix_timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let url = format!("{}?{}", api_url, unix_timestamp);
    let mut headers = HeaderMap::new();
    headers.insert("Pragma", HeaderValue::from_static("no-cache"));
    headers.insert("Referer", HeaderValue::from_static("https://www.oref.org.il/11226-he/pakar.aspx"));
    headers.insert("X-Requested-With", HeaderValue::from_static("XMLHttpRequest"));
    headers.insert(
        "User-Agent",
        HeaderValue::from_static("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_13_6) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/75.0.3770.100 Safari/537.36"),
    );

    let client = reqwest::Client::new();
    let response = client.get(&url).headers(headers).send().await;

    match response {
        Ok(res) if res.status() == reqwest::StatusCode::OK => {
            let body = res.text().await?;

            if body.trim().is_empty() {
                return Ok(json!({
                    "type": "none",
                    "cities": []
                }));
            }

            let json: Value = serde_json::from_str(&body).map_err(|e| {
                format!("Failed to parse the response body as JSON: {}. Body was: {}", e, body)
            })?;

            if json.get("data").is_none() {
                return Ok(json!({
                    "type": "none",
                    "cities": []
                }));
            }

            Ok(json)
        }
        Ok(res) => {
            log::error!("Failed to retrieve alerts from HFC API: {} {}", res.status().as_u16(), res.status().canonical_reason().unwrap_or("Unknown"));
            // Return a default JSON object indicating failure
            Ok(json!({
                "type": "none",
                "cities": []
            }))
        }
        Err(e) => {
            log::error!("Error making request to HFC API: {}", e);
            // Return a default JSON object indicating failure
            Ok(json!({
                "type": "none",
                "cities": []
            }))
        }
    }
}


// Async function to extract the alert data from the JSON
async fn extract_alert_from_json(json: serde_json::Value) -> Result<AlertResult, Box<dyn std::error::Error>> {
    // Check if it is an array (History JSON)
    if json.is_array() {
        return extract_alert_from_history_json(json).await;
    }

    let alert_data: Alert = serde_json::from_value(json)?;

    let mut alert = AlertResult {
        alert_type: "none".to_string(),
        cities: vec![],
        instructions: alert_data.instructions,
    };

    if let Some(cities) = alert_data.cities {
        for mut city in cities {
            city = city.trim().to_string();
            // Skip "test" alerts (Hebrew check)
            if city.contains("בדיקה") {
                continue;
            }
            if !alert.cities.contains(&city) {
                alert.cities.push(city);
            }
        }
    }

    if let Some(category) = alert_data.category {
        alert.alert_type = get_alert_type_by_category(&category);
    }

    Ok(alert)
}

// Extract alert from history JSON
async fn extract_alert_from_history_json(json: serde_json::Value) -> Result<AlertResult, Box<dyn std::error::Error>> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let mut alert = AlertResult {
        alert_type: "none".to_string(),
        cities: vec![],
        instructions: None,
    };

    let history: Vec<HistoryAlert> = serde_json::from_value(json)?;

    for item in history {
        if let (Some(alert_date), Some(city), Some(category)) = (item.alertDate, item.data, item.category) {
            let alert_time = (chrono::DateTime::parse_from_rfc3339(&alert_date)?.timestamp() as u64) / 1000;

            if now - alert_time > 120 {
                continue;
            }

            let trimmed_city = city.trim().to_string();

            if trimmed_city.contains("בדיקה") {
                continue;
            }

            if !alert.cities.contains(&trimmed_city) {
                alert.cities.push(trimmed_city);
            }

            alert.alert_type = get_alert_type_by_historical_category(&category);
        }
    }

    Ok(alert)
}

// Function to get alert type by category
fn get_alert_type_by_category(category: &str) -> String {
    match category.parse::<u32>() {
        Ok(1) => "missiles".to_string(),
        Ok(2) => "general".to_string(),
        Ok(3) => "earthQuake".to_string(),
        Ok(4) => "radiologicalEvent".to_string(),
        Ok(5) => "tsunami".to_string(),
        Ok(6) => "hostileAircraftIntrusion".to_string(),
        Ok(7) => "hazardousMaterials".to_string(),
        Ok(13) => "terroristInfiltration".to_string(),
        Ok(101) => "missilesDrill".to_string(),
        Ok(102) => "generalDrill".to_string(),
        Ok(103) => "earthQuakeDrill".to_string(),
        Ok(104) => "radiologicalEventDrill".to_string(),
        Ok(105) => "tsunamiDrill".to_string(),
        Ok(106) => "hostileAircraftIntrusionDrill".to_string(),
        Ok(107) => "hazardousMaterialsDrill".to_string(),
        Ok(113) => "terroristInfiltrationDrill".to_string(),
        _ => "unknown".to_string(),
    }
}

// Function to get alert type by historical category
fn get_alert_type_by_historical_category(category: &str) -> String {
    match category.parse::<u32>() {
        Ok(1) => "missiles".to_string(),
        Ok(2) => "hostileAircraftIntrusion".to_string(),
        Ok(3) => "general".to_string(),
        Ok(4) => "general".to_string(),
        Ok(7) => "earthQuake".to_string(),
        Ok(9) => "radiologicalEvent".to_string(),
        Ok(10) => "terroristInfiltration".to_string(),
        Ok(11) => "tsunami".to_string(),
        Ok(12) => "hazardousMaterials".to_string(),
        _ => "unknown".to_string(),
    }
}


