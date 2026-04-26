pub fn error(message: impl AsRef<str>) {
    eprint!("\r\n{}\r\n", message.as_ref());
}
