#[derive(Debug, Clone)]
pub struct PublishRequest<'a> {
    pub title: &'a str,
    pub author_name: Option<&'a str>,
    pub author_url: Option<&'a str>,
    pub html: &'a str,
}

#[allow(async_fn_in_trait)]
pub trait PreviewPublisher: Send + Sync {
    async fn publish(&self, req: &PublishRequest<'_>) -> anyhow::Result<Option<String>>;
}

#[derive(Debug, Clone, Default)]
pub struct NoopPublisher;

impl PreviewPublisher for NoopPublisher {
    async fn publish(&self, _req: &PublishRequest<'_>) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
}
