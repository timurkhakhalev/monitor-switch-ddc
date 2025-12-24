#[derive(Debug, Clone)]
pub struct MenuSpec {
    pub items: Vec<MenuItem>,
}

impl MenuSpec {
    pub fn new(items: Vec<MenuItem>) -> Self {
        Self { items }
    }
}

#[derive(Debug, Clone)]
pub enum MenuItem {
    Header(String),
    Separator,
    Action {
        id: u16,
        title: String,
        checked: bool,
        enabled: bool,
    },
}
