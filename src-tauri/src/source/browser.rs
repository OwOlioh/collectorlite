use async_trait::async_trait;
use scraper::{Html, Selector};
use sha2::{Digest, Sha256};
use url::Url;

use crate::error::AppError;
use crate::models::{CollectionInfo, ExternalItem};
use crate::source::SourceAdapter;

pub struct BrowserBookmarkClient;

impl BrowserBookmarkClient {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self
    }

    fn resolve_favicon_url(page_url: &str) -> Option<String> {
        let parsed = Url::parse(page_url).ok()?;
        let domain = parsed.host_str()?;
        Some(format!(
            "https://www.google.com/s2/favicons?domain={domain}&sz=32"
        ))
    }

    pub fn parse_bookmarks_html(html: &str) -> Result<Vec<ExternalItem>, AppError> {
        let document = Html::parse_document(html);
        let selector = Selector::parse("a").map_err(|e| AppError::Other(e.to_string()))?;

        let mut items = Vec::new();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        for element in document.select(&selector) {
            let href = element.value().attr("href").unwrap_or("");
            if href.is_empty()
                || href.starts_with("javascript:")
                || href.starts_with("place:")
            {
                continue;
            }

            let title = element
                .text()
                .collect::<Vec<_>>()
                .join("")
                .trim()
                .to_string();
            if title.is_empty() {
                continue;
            }

            let add_date = element
                .value()
                .attr("add_date")
                .and_then(|s| s.parse::<i64>().ok());

            let icon = Self::resolve_favicon_url(href);

            let folder_names = Self::build_folder_names(&element);

            let mut hasher = Sha256::new();
            hasher.update(href.as_bytes());
            let hash = format!("{:x}", hasher.finalize())[..16].to_string();

            let item = ExternalItem {
                source: "browser".into(),
                external_id: format!("bk_{}", hash),
                source_url: href.to_string(),
                title,
                description: String::new(),
                cover_url: icon,
                cover_local_path: None,
                author_name: None,
                author_id: None,
                partition_name: None,
                published_at: add_date,
                duration: None,
                favorite_time: Some(add_date.unwrap_or(now)),
                extra: serde_json::json!({ "folder_tags": folder_names }),
            };

            items.push(item);
        }

        Ok(items)
    }

    fn build_folder_names(element: &scraper::ElementRef) -> Vec<String> {
        let mut parts: Vec<String> = Vec::new();

        let mut current = element.parent();
        while let Some(parent) = current {
            if let Some(parent_elem) = parent.value().as_element() {
                if parent_elem.name() == "dl" {
                    if let Some(dt_parent) = parent.parent() {
                        if let Some(dt_elem) = dt_parent.value().as_element() {
                            if dt_elem.name() == "dt" {
                                for child in dt_parent.children() {
                                    if let Some(child_elem) = child.value().as_element() {
                                        if child_elem.name() == "h3" {
                                            let text = child
                                                .descendants()
                                                .filter_map(|n| {
                                                    if n.value().is_text() {
                                                        n.value().as_text().map(|t| t.as_ref())
                                                    } else {
                                                        None
                                                    }
                                                })
                                                .collect::<Vec<_>>()
                                                .join("")
                                                .trim()
                                                .to_string();
                                            if !text.is_empty() {
                                                parts.push(text);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            current = parent.parent();
        }

        parts.reverse();
        parts
    }
}

#[async_trait]
impl SourceAdapter for BrowserBookmarkClient {
    async fn list_collections(&self) -> Result<Vec<CollectionInfo>, AppError> {
        Ok(vec![CollectionInfo {
            source: "browser".into(),
            id: "browser-bookmarks".into(),
            title: "浏览器书签".into(),
            owner: None,
            count: 0,
            url: None,
        }])
    }

    async fn resolve_collection(&self, _input: &str) -> Result<CollectionInfo, AppError> {
        Ok(CollectionInfo {
            source: "browser".into(),
            id: "browser-bookmarks".into(),
            title: "浏览器书签".into(),
            owner: None,
            count: 0,
            url: None,
        })
    }

    async fn fetch_collection(
        &self,
        _collection: &CollectionInfo,
    ) -> Result<Vec<ExternalItem>, AppError> {
        Ok(vec![])
    }

    async fn enrich_items(&self, items: &[ExternalItem]) -> Result<Vec<ExternalItem>, AppError> {
        Ok(items.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_bookmarks() {
        let html = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks</H1>
<DL><p>
    <DT><H3>技术</H3>
    <DL><p>
        <DT><A HREF="https://www.rust-lang.org" ADD_DATE="1700000000">Rust 官网</A>
        <DT><A HREF="https://react.dev" ADD_DATE="1700000001">React 文档</A>
    </DL><p>
    <DT><H3>工具</H3>
    <DL><p>
        <DT><A HREF="https://github.com" ADD_DATE="1700000002">GitHub</A>
    </DL><p>
</DL><p>"#;

        let items = BrowserBookmarkClient::parse_bookmarks_html(html).unwrap();
        assert_eq!(items.len(), 3);

        let rust_item = items.iter().find(|i| i.title == "Rust 官网").unwrap();
        assert_eq!(rust_item.source, "browser");
        assert_eq!(rust_item.source_url, "https://www.rust-lang.org");
        assert_eq!(rust_item.description, "");
        assert_eq!(rust_item.partition_name, None);
        assert!(rust_item.external_id.starts_with("bk_"));
        let tags: Vec<String> = rust_item.extra["folder_tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(tags, vec!["技术"]);

        let github_item = items.iter().find(|i| i.title == "GitHub").unwrap();
        let github_tags: Vec<String> = github_item.extra["folder_tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(github_tags, vec!["工具"]);
    }

    #[test]
    fn test_skip_javascript_links() {
        let html = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks</H1>
<DL><p>
    <DT><A HREF="javascript:void(0)">Bookmarklet</A>
    <DT><A HREF="https://example.com">Real Site</A>
</DL><p>"#;

        let items = BrowserBookmarkClient::parse_bookmarks_html(html).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Real Site");
    }

    #[test]
    fn test_nested_folders() {
        let html = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks</H1>
<DL><p>
    <DT><H3>编程</H3>
    <DL><p>
        <DT><H3>前端</H3>
        <DL><p>
            <DT><A HREF="https://vuejs.org">Vue.js</A>
        </DL><p>
    </DL><p>
</DL><p>"#;

        let items = BrowserBookmarkClient::parse_bookmarks_html(html).unwrap();
        assert_eq!(items.len(), 1);
        let tags: Vec<String> = items[0].extra["folder_tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(tags, vec!["编程", "前端"]);
    }
}