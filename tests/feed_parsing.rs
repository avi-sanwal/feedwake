use feedwake::feed::parse_feed_bytes;

#[test]
fn parses_rss_items_with_title_url_and_description() {
    let rss = br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>NSE News</title>
    <item>
      <title>Reliance Industries Limited</title>
      <link>https://example.com/reliance.pdf</link>
      <description>Board Meeting Intimation</description>
      <pubDate>Sat, 09 May 2026 11:47:34 +0530</pubDate>
    </item>
  </channel>
</rss>"#;

    let items = parse_feed_bytes(rss, "NSE Announcements", "https://example.com/feed.xml", 10)
        .expect("parse rss");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].source_name, "NSE Announcements");
    assert_eq!(items[0].title, "Reliance Industries Limited");
    assert_eq!(items[0].url, "https://example.com/reliance.pdf");
    assert_eq!(
        items[0].description.as_deref(),
        Some("Board Meeting Intimation")
    );
}

#[test]
fn respects_max_items_per_feed() {
    let rss = br#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Feed</title>
    <item><title>One</title><link>https://example.com/1</link></item>
    <item><title>Two</title><link>https://example.com/2</link></item>
  </channel>
</rss>"#;

    let items =
        parse_feed_bytes(rss, "Feed", "https://example.com/feed.xml", 1).expect("parse rss");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "One");
}
