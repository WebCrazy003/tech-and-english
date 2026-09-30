use serde::Serialize;
use tauri::State;

use super::CmdResult;
use crate::db::repo::articles::{self, ArticleListItem};
use crate::news::SaveBody;
use crate::news::body::FetchedHtml;
use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderArticle {
    #[serde(flatten)]
    pub item: ArticleListItem,
    /// Sanitized HTML from Readability (only when `bodyStatus == "ok"`).
    pub body_html: Option<String>,
}

#[tauri::command]
pub async fn get_reader_article(state: State<'_, AppState>, id: i64) -> CmdResult<ReaderArticle> {
    state
        .db
        .call(move |c| {
            let item = articles::get_item(c, id)?;
            let (_, body_html) = articles::body_html(c, id)?;
            Ok(ReaderArticle { item, body_html })
        })
        .await
}

#[tauri::command]
pub async fn fetch_article_html(state: State<'_, AppState>, article_id: i64) -> CmdResult<FetchedHtml> {
    state.news.fetch_article_html(article_id).await
}

#[tauri::command]
pub async fn save_article_body(state: State<'_, AppState>, input: SaveBody) -> CmdResult<ArticleListItem> {
    state.news.save_article_body(input).await
}
