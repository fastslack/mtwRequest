use mtw_core::MtwError;
use serde::de::DeserializeOwned;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::client::MtwHttpClient;

/// Auto-paginating iterator over HTTP API results.
pub struct Paginator<T> {
    client: Arc<MtwHttpClient>,
    next_url: Option<String>,
    page: u32,
    items_field: String,
    _phantom: PhantomData<T>,
}

impl<T: DeserializeOwned> Paginator<T> {
    /// Create a new paginator starting from the given URL.
    pub fn new(client: Arc<MtwHttpClient>, start_url: impl Into<String>) -> Self {
        Self {
            client,
            next_url: Some(start_url.into()),
            page: 0,
            items_field: "items".to_string(),
            _phantom: PhantomData,
        }
    }

    /// Set the JSON field name that contains the items array.
    pub fn items_field(mut self, field: impl Into<String>) -> Self {
        self.items_field = field.into();
        self
    }

    /// Fetch the next page of results. Returns None when there are no more pages.
    pub async fn next_page(&mut self) -> Result<Option<Vec<T>>, MtwError> {
        let url = match &self.next_url {
            Some(url) => url.clone(),
            None => return Ok(None),
        };

        let response = self.client.get(&url).await?;

        if !response.is_success() {
            return Err(MtwError::Transport(format!(
                "pagination request failed with status {}",
                response.status
            )));
        }

        // Update next URL from pagination info
        self.next_url = response.pagination.as_ref().and_then(|p| p.next.clone());
        self.page += 1;

        // Try to extract items from JSON response
        let body: serde_json::Value = response.json()?;

        let items_value = if let Some(items) = body.get(&self.items_field) {
            items.clone()
        } else if body.is_array() {
            body
        } else {
            return Err(MtwError::Codec(format!(
                "could not find items field '{}' in response",
                self.items_field
            )));
        };

        let items: Vec<T> =
            serde_json::from_value(items_value).map_err(|e| MtwError::Codec(e.to_string()))?;

        Ok(Some(items))
    }

    /// Get the current page number (1-indexed after first fetch).
    pub fn current_page(&self) -> u32 {
        self.page
    }

    /// Check if there are more pages available.
    pub fn has_more(&self) -> bool {
        self.next_url.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paginator_creation() {
        let client = Arc::new(MtwHttpClient::new());
        let paginator: Paginator<serde_json::Value> =
            Paginator::new(client, "https://api.example.com/items");
        assert_eq!(paginator.current_page(), 0);
        assert!(paginator.has_more());
    }

    #[test]
    fn test_paginator_items_field() {
        let client = Arc::new(MtwHttpClient::new());
        let paginator: Paginator<serde_json::Value> =
            Paginator::new(client, "https://api.example.com/items").items_field("data");
        assert_eq!(paginator.items_field, "data");
    }
}
