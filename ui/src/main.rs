slint::include_modules!();

fn main() {
    let main_window = MainWindow::new().unwrap();
    let w_main_window = main_window.weak();

    std::thread::spawn(move || {
        let get = ureq::get("http://localhost:8080/api/tabledump")
            .call()
            .unwrap()
            .json::<>();
    })
    .unwrap();

    main_window.run().unwrap();
}
