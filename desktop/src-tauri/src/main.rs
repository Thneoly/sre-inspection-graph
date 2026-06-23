// 阻止 Windows release 模式弹出额外控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    sre_graph_desktop_lib::run();
}
