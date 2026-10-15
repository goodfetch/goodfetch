from goodfetch_collie_client import Browser, Page
from goodfetch_collie_client._sessions import Session
import os

async def login_human_pause() -> None:
    browser = await Browser.launch(headless=False)
    page = await browser.new_page()
    await page.goto('https://quotes.toscrape.com/login')
    await page.fill('input[name="username"]', 'your_username')
    await page.fill('input[name="password"]', os.getenv('LOGIN_PASSWORD'))
    await page.click('input[type="submit"]')
    await page.wait_for_selector('a[href="/logout"]')
    session = await page.context.storage_state(path='session.json')
    await browser.close()

async def check_login() -> None:
    browser = await Browser.launch(headless=True)
    page = await browser.new_page()
    await page.context.storage_state(path='session.json')
    await page.goto('https://quotes.toscrape.com/')
    try:
        await page.wait_for_selector('a[href="/logout"]')
        print('Login successful')
    except Exception as e:
        print('Session expired')
    await browser.close()