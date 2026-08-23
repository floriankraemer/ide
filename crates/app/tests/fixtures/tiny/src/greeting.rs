pub fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

pub fn shout_loudly(name: &str) -> String {
    greet(name).to_uppercase()
}
