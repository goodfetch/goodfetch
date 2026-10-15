// Program to automate the task using Goodfetch's SDK

use goodfetch_collie_client::{Run, Session, Page};
use goodfetch_foxhound::{Foxhound, ConnectOptions};
use futures_util::future::join_all;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut foxhound = Foxhound::connect(ConnectOptions::default()).await?;
    let mut session = foxhound.new_session().await?;
    let mut page = session.new_page().await?;

    page.goto("https://quotes.toscrape.com/search.aspx").await?;

    // Choose the author "Albert Einstein"
    page.select_option("#author", "Albert Einstein").await?;
    page.wait_for_navigation().await?;

    // Choose the tag "inspirational"
    page.select_option("#tag", "inspirational").await?;
    page.click("#submit_button").await?;
    page.wait_for_navigation().await?;

    // Fetch all quotes
    let quotes = page.evaluate("() => {
        let quotes = [];
        document.querySelectorAll('.quote').forEach(quote => {
            quotes.push({
                text: quote.querySelector('.text').innerText,
                author: quote.querySelector('.author').innerText,
                tag: quote.querySelector('.tag').innerText.split(', ').map(tag => tag.trim())
            });
        });
        return quotes;
    }").await?;

    if quotes.is_empty() {
        println!("No quotes found.");
    } else {
        for quote in quotes {
            println!("Text: {:?}, Author: {:?}, Tag: {:?}", quote.text, quote.author, quote.tag);
        }
    }

    session.close().await?;
    foxhound.close().await?;

    Ok(()
}
