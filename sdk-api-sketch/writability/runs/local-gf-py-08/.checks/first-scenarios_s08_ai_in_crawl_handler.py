# Import necessary libraries
from goodfetch_collie_client import *
from goodfetch_foxhound import *

# Define the main function
def main():
    # Initialize the crawler
    crawler = Crawler()
    
    # Define the categories to crawl
    categories = ['fiction', 'nonfiction', 'science']
    
    # Start crawling each category
    for category in categories:
        crawler.start_category(category)
        
        # Follow 'next' pages
        while crawler.has_next_page():
            crawler.go_to_next_page()
            
        # Extract book details
        books = crawler.get_books()
        for book in books:
            print(f'Title: {book.title}, Price: {book.price}, Stock: {book.stock}, Rating: {book.rating}')

# Run the main function
if __name__ == '__main__':
    main()
