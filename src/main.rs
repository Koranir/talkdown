mod app;
mod checker;
mod codex;
mod document;
mod edit;
mod event_stream;
mod file_watch;
mod model;
mod speech;

fn main() -> iced::Result {
    app::run()
}
