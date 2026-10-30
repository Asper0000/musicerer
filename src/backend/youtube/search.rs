pub fn term(query: &str, count: usize) -> String {
    format!("ytsearch{count}:{query}")
}