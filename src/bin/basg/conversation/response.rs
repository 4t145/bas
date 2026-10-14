use super::*;

pub(crate) struct Response {
    pub text: String,
    pub user_item: Option<usize>,
    pub latest: bool,
}

impl Conversation {
    /// A response's controls belong to its last item, before the next user message.
    pub fn response(&self, index: usize) -> Option<Response> {
        let item = self.items.get(index)?;
        if is_user_message(item) {
            return None;
        }
        let user_item = self.items[..index].iter().rposition(is_user_message);
        let start = user_item.map_or(0, |index| index + 1);
        let end = self.items[index + 1..]
            .iter()
            .position(is_user_message)
            .map_or(self.items.len(), |offset| index + offset + 1);
        if index + 1 != end {
            return None;
        }
        Some(Response {
            text: self.items[start..end]
                .iter()
                .filter_map(|item| match item {
                    Item::Message { message, .. } if message.role == Role::Agent => {
                        let text = message.plain_text();
                        (!text.trim().is_empty()).then_some(text)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
            user_item,
            latest: end == self.items.len(),
        })
    }
}

fn is_user_message(item: &Item) -> bool {
    matches!(item, Item::Message { message, .. } if message.role == Role::User)
}
