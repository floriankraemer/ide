pub fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

pub fn shout(name: &str) -> String {
    greet(name).to_uppercase()
}
