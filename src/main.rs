mod api;
mod api_client;
mod api_config;
mod api_transport;
mod app_identity;
mod appearance;
mod configured_ports;
mod console_buffer;
mod console_search;
mod discovery;
mod editor;
mod editor_layout;
mod editor_service;
mod external;
mod java_edit;
mod launch;
mod lifecycle;
mod model;
mod output;
mod preferences;
mod presentation;
mod profile;
mod project_manifest;
mod runtime;
mod runtime_ports;
mod service;
mod store;
mod time_display;
mod ui;
mod windows;
fn main() -> eframe::Result {
    let args: Vec<_> = std::env::args().collect();
    let action = match args.get(1).map(String::as_str) {
        Some("--edition") => {
            println!("{}", app_identity::edition());
            return Ok(());
        }
        Some("--api") => Some(api_client::cli(&args)),
        Some("--mcp") => Some(api_client::mcp()),
        Some("--logs") => Some(api_client::follow(&args)),
        Some("--serve") => Some(serve()),
        _ => None,
    };
    if let Some(result) = action {
        if let Err(e) = result {
            eprintln!("[dev start] {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.get(1).is_some_and(|a| a == "--import-profile") {
        let result = (|| -> Result<(), String> {
            let path = args.get(2).ok_or("--import-profile 需要文件路径")?;
            let mut service = service::Service::load()?;
            service.import_profile(std::path::Path::new(path))?;
            for p in &service.catalog.projects {
                for s in &p.scripts {
                    println!("{} · {} · id={}", p.name, s.name, s.id);
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("[dev start] {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("DevStart")
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!(
                    "../assets/devstart-window-icon.png"
                ))
                .expect("embedded DevStart icon is valid PNG"),
            )
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "DevStart",
        options,
        Box::new(|cc| Ok(Box::new(ui::DevStart::new(cc)))),
    )
}
#[cfg(test)]
mod tests;

static SERVE_STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
extern "C" fn stop_server(_: libc::c_int) {
    SERVE_STOP.store(true, std::sync::atomic::Ordering::Relaxed);
}
fn serve() -> Result<(), String> {
    let mut service = service::Service::load()?;
    let api = api_transport::Server::bind(&service.store.root)?;
    // The handler only sets an atomic flag; process cleanup stays in normal Rust control flow.
    unsafe {
        libc::signal(
            libc::SIGTERM,
            stop_server as *const () as libc::sighandler_t,
        );
        libc::signal(libc::SIGINT, stop_server as *const () as libc::sighandler_t);
    }
    while !SERVE_STOP.load(std::sync::atomic::Ordering::Relaxed) {
        service.poll();
        api.poll(&mut service);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok(())
}
