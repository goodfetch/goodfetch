// Import necessary crates and modules
use std::env;
use futures_util::future::FutureExt;
use goodfetch_collie::sessions::Session;
use goodfetch_foxhound::crawl::Crawl;

#[tokio::main]
async fn main() {
    // Get the password from the environment variable
    let password = env::var("QUOTES_PASSWORD").expect("QUOTES_PASSWORD must be set");

    // Start a visible browser session
    let mut session = Session::new_visible().await.unwrap();

    // Navigate to the login page
    session.navigate("https://quotes.toscrape.com/login").await.unwrap();

    // Fill the username and password fields
    session.fill_input("#username", "your_username").await.unwrap();
    session.fill_input("#password", &password).await.unwrap();

    // Wait for the user to press Login
    session.wait_for_element("#login_form").await.unwrap();

    // Keep the logged-in state under a name
    session.save_state("logged_in_state").await.unwrap();

    // In a second, headless run, reuse that state
    let mut headless_session = Session::new_headless().await.unwrap();
    headless_session.load_state("logged_in_state").await.unwrap();

    // Navigate to the main page
    headless_session.navigate("https://quotes.toscrape.com/").await.unwrap();

    // Confirm that the page shows a Logout link
    let logout_link_exists = headless_session.element_exists("#logout").await.unwrap();

    if logout_link_exists {
        println!("Logout link is present");
    } else {
        println!("Logout link is not present, state may have expired");
    }
}
