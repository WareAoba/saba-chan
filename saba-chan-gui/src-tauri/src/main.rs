// 릴리스에서 콘솔 창을 띄우지 않는다 (Windows)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    saba_chan_gui_app::run()
}
