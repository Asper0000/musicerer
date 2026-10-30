pub fn term(query: &str, count: usize) -> String {
    format!("scsearch{count}:{query}")
}