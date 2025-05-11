use leptos::prelude::*;

fn main() {
    leptos::mount::mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    let (count, set_count) = signal(0);

    view! {
        <button 
        on:click=move |_| set_count.update(|x| {*x += 1})
        class:red=move || count.get() % 2 == 1 >
            "Click me:"
            {count}
        </button>
    }
}
