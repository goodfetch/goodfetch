// Colly async-mode bench adapter (ADR 0025, issue 181).
// Speaks the bench JSON-line adapter protocol (hello, prepare, run, close)
// on stdin/stdout using Colly v2.1.0 in async mode (`colly.Async(true)`).
package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"regexp"
	"strings"
	"sync"

	"github.com/gocolly/colly/v2"
)

type requestMsg struct {
	ID     any            `json:"id"`
	Method string         `json:"method"`
	Params map[string]any `json:"params"`
}

type productRecord struct {
	ID    string `json:"id"`
	Name  string `json:"name"`
	Price string `json:"price"`
}

var priceRe = regexp.MustCompile(`([0-9]+\.[0-9]{2})`)

func reply(id any, result any) {
	b, _ := json.Marshal(map[string]any{"version": 1, "id": id, "result": result})
	fmt.Println(string(b))
}

func replyErr(id any, err string) {
	b, _ := json.Marshal(map[string]any{"version": 1, "id": id, "error": err})
	fmt.Println(string(b))
}

func newCollector(provingAddr string) *colly.Collector {
	c := colly.NewCollector(colly.Async(true))
	_ = c.Limit(&colly.LimitRule{DomainGlob: "*", Parallelism: 16})
	c.WithTransport(&http.Transport{
		Proxy: func(_ *http.Request) (*url.URL, error) {
			return url.Parse("http://" + provingAddr)
		},
	})
	return c
}

func main() {
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 0, 64*1024), 4*1024*1024)
	address := ""

	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" {
			continue
		}
		var msg requestMsg
		if err := json.Unmarshal([]byte(line), &msg); err != nil {
			continue
		}
		switch msg.Method {
		case "hello":
			reply(msg.ID, map[string]any{
				"capabilities":    []string{"crawl", "extract"},
				"max_concurrency": 16,
			})
		case "prepare":
			if pg, ok := msg.Params["proving_ground"].(map[string]any); ok {
				if addr, ok := pg["address"].(string); ok {
					address = addr
				}
			}
			reply(msg.ID, map[string]any{})
		case "run":
			task, ok := msg.Params["task"].(map[string]any)
			if !ok {
				replyErr(msg.ID, "invalid task payload")
				continue
			}
			kind, _ := task["kind"].(string)
			startURL, _ := task["start_url"].(string)
			c := newCollector(address)
			var mu sync.Mutex
			seen := map[string]productRecord{}
			if kind == "extract" {
				c.OnHTML("article.product", func(e *colly.HTMLElement) {
					rec := productRecord{
						ID:    e.Attr("data-id"),
						Name:  strings.TrimSpace(e.ChildText("h1.name")),
						Price: e.ChildAttr("span.price", "data-price"),
					}
					mu.Lock()
					seen[rec.ID] = rec
					mu.Unlock()
				})
			} else if kind == "crawl" {
				c.OnHTML("li.product", func(e *colly.HTMLElement) {
					id := e.Attr("data-id")
					name := strings.TrimSpace(e.ChildText("a"))
					price := priceRe.FindString(e.ChildText("span.price"))
					if id != "" {
						mu.Lock()
						seen[id] = productRecord{ID: id, Name: name, Price: price}
						mu.Unlock()
					}
				})
				c.OnHTML("a[rel=next]", func(e *colly.HTMLElement) {
					next := e.Request.AbsoluteURL(e.Attr("href"))
					if next != "" {
						_ = e.Request.Visit(next)
					}
				})
			} else {
				reply(msg.ID, map[string]any{"status": "unsupported", "reason": "unsupported kind"})
				continue
			}
			if err := c.Visit(startURL); err != nil {
				replyErr(msg.ID, err.Error())
				continue
			}
			c.Wait()
			records := make([]productRecord, 0, len(seen))
			for _, rec := range seen {
				records = append(records, rec)
			}
			reply(msg.ID, map[string]any{"status": "success", "records": records})
		case "close":
			reply(msg.ID, map[string]any{})
			return
		default:
			replyErr(msg.ID, "unknown method "+msg.Method)
		}
	}
}
