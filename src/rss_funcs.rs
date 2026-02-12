use rss::Channel;
use std::error::Error;

pub async fn get_feed(url: &str) -> Result<Vec<(String, String, String)>, Box<dyn Error + Send + Sync>> {
    let content = reqwest::get(url).await?.bytes().await?;
    let channel = Channel::read_from(&content[..])?;
    
    let items = channel
        .items()
        .iter()
        .map(|item| {
            let title = item.title().unwrap_or("No Title").to_string();
            let date = item.pub_date().unwrap_or("N/A");
            let short_date = date.split(' ').skip(1).take(2).collect::<Vec<_>>().join(" ");
            
            let raw_desc = item.description().unwrap_or("No description available.").to_string();
            let decoded = html_escape::decode_html_entities(&raw_desc).to_string();
            
            let clean_desc = decoded
                .replace("<p>", "").replace("</p>", "")
                .replace("<br>", "\n").replace("</br>", "\n")
                .replace("<em>", "").replace("</em>", "")
                .replace("<strong>", "").replace("</strong>", "");
            
            (title, short_date, clean_desc)
        })
        .collect();

    Ok(items)
}