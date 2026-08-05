//! Agent context management.
//!
//! This module handles context-related operations including
//! user input preparation, overflow handling, and context compaction.

use super::*;

impl Agent {
    pub(super) async fn prepare_user_input(
        &self,
        input: &str,
        images: &[Option<PastedImage>],
    ) -> Result<PreparedUserInput> {
        let input = clean_user_visible_text(input);
        let binary_images = images
            .iter()
            .filter_map(|image| match image {
                Some(PastedImage::Binary(image)) => Some(image),
                _ => None,
            })
            .collect::<Vec<_>>();
        let path_images = images
            .iter()
            .filter_map(|image| match image {
                Some(PastedImage::Path(path)) => Some(path.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let absolute_image_paths = resolve_pasted_image_paths(images, &self.paths);
        let temp_paths = absolute_image_paths
            .iter()
            .filter_map(|path| path.clone())
            .collect::<Vec<_>>();
        let input = rewrite_image_placeholders_with_paths(&input, &absolute_image_paths);
        let content = if !binary_images.is_empty() && !self.current_model_supports_vision() {
            self.describe_images_with_vision_provider(&input, &binary_images)
                .await?
        } else {
            input
        };

        let message = if !binary_images.is_empty() && self.current_model_supports_vision() {
            let mut parts = vec![ChatContentPart::Text {
                text: content.clone(),
            }];
            parts.extend(binary_images.iter().map(|image| ChatContentPart::ImageUrl {
                image_url: ImageUrlContent {
                    url: image.data_url(),
                },
            }));
            ChatMessage {
                role: "user".to_string(),
                content: Some(ChatContent::Parts(parts)),
                tool_call_id: None,
                tool_calls: None,
            }
        } else {
            ChatMessage::plain("user", &content)
        };

        let mut hints = Vec::new();
        if !temp_paths.is_empty() {
            let hint = if temp_paths.len() == 1 {
                format!(
                    "用户粘贴了 1 张剪贴板图片，已保存到临时文件：{}\n你可以使用 vision_analyze 工具对此图片进行更详细的分析。",
                    temp_paths[0]
                )
            } else {
                let list = temp_paths
                    .iter()
                    .enumerate()
                    .map(|(index, path)| format!("  [Image {}] {}", index + 1, path))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "用户粘贴了 {} 张剪贴板图片，已保存到临时文件：\n{}\n你可以使用 vision_analyze 工具对这些图片进行更详细的分析。",
                    temp_paths.len(),
                    list
                )
            };
            hints.push(ChatMessage::system(hint));
        }
        if !path_images.is_empty() {
            let list = path_images
                .iter()
                .enumerate()
                .map(|(index, path)| format!("  [Image {}] {}", index + 1, path))
                .collect::<Vec<_>>()
                .join("\n");
            hints.push(ChatMessage::system(format!(
                "用户粘贴了 {} 张本地图片路径：\n{}\n你可以使用 vision_analyze 工具读取并分析这些图片。",
                path_images.len(),
                list
            )));
        }

        Ok(PreparedUserInput {
            content,
            message,
            hints,
        })
    }

    pub async fn handle_overflow_after_turn<F>(
        &self,
        context_tokens: u64,
        on_event: F,
    ) -> Result<Option<ChatResult>>
    where
        F: FnMut(AgentEvent) -> Result<()>,
    {
        let mut on_event = on_event;
        let Some(compact) = self.handle_overflow(context_tokens, &mut on_event).await? else {
            return Ok(None);
        };
        self.state.add_auxiliary_usage(&compact.usage, "system", "memory_compact")?;
        Ok(Some(ChatResult {
            content: String::new(),
            reasoning: None,
            usage: Some(compact.usage),
            usage_estimated: compact.usage_estimated,
            tool_calls: Vec::new(),
            provider_id: None,
            model: None,
        }))
    }

    pub async fn compact_now<F>(&self, on_event: F) -> Result<Option<ChatResult>>
}
