use std::cmp;
use xcb::{x, Connection, Event, Xid};

#[derive(Clone)]
struct WindowData {
    window: x::Window,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

impl WindowData {
    fn new(window: x::Window) -> Self {
        WindowData {
            window,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        }
    }
}

fn update_window(conn: &Connection, data: &WindowData) {
    conn.send_request(&x::ConfigureWindow {
        window: data.window,
        value_list: &[
            x::ConfigWindow::X(data.x),
            x::ConfigWindow::Y(data.y),
            x::ConfigWindow::Width(data.w),
            x::ConfigWindow::Height(data.h),
        ],
    });
}

// TODO: Optimize, too many copies
fn master_tiling(
    conn: &Connection,
    screen: &x::Screen,
    mut windows: Vec<WindowData>,
) -> Vec<WindowData> {
    if windows.len() == 0 {
        return windows;
    }

    let screen_res = (
        screen.width_in_pixels() as u32,
        screen.height_in_pixels() as u32,
    );

    windows[0].x = 0;
    windows[0].y = 0;
    windows[0].w = (screen_res.0 as f32 * 0.6) as u32;
    windows[0].h = screen_res.1;

    if windows.len() == 1 {
        windows[0].w = screen_res.0;
        windows[0].h = screen_res.1;
        update_window(&conn, &windows[0]);
    } else if windows.len() == 2 {
        // master window width becomes 60% of screen width
        update_window(&conn, &windows[0]);

        // first slave
        windows[1].x = windows[0].w as i32;
        windows[1].y = 0;
        windows[1].w = screen_res.0 - windows[1].x as u32;
        windows[1].h = screen_res.1;
        update_window(&conn, &windows[1]);
    } else {
        // setup slaves
        let num_slaves = windows.len() - 1; // ignore master window

        let slave_height = (screen_res.1 as f32 / num_slaves as f32) as u32;

        windows = windows
            .iter()
            .enumerate()
            .map(|(i, win)| {
                let mut win = win.to_owned();
                if i >= 1 {
                    win.x = windows[0].w as i32;
                    win.y = ((i - 1) as u32 * slave_height) as i32;
                    win.w = screen_res.0 - windows[0].w;
                    win.h = slave_height;
                };

                update_window(&conn, &win);

                win
            })
            .collect();
    }

    windows
}

fn main() {
    let (conn, _) = Connection::connect(None).expect("Failed to connect to X server");
    println!("Connected to X server");

    let setup = conn.get_setup();
    let screen = setup.roots().next().expect("Failed to get root window");
    let root_window = screen.root();

    let modkey = x::ModMask::CONTROL;

    conn.send_request(&x::ChangeWindowAttributes {
        window: root_window,
        value_list: &[x::Cw::EventMask(x::EventMask::SUBSTRUCTURE_NOTIFY)],
    });

    conn.send_request(&x::GrabKey {
        owner_events: true,
        grab_window: root_window,
        modifiers: modkey,
        key: 67, // F1
        pointer_mode: x::GrabMode::Async,
        keyboard_mode: x::GrabMode::Async,
    });

    conn.send_request(&x::GrabButton {
        owner_events: true,
        grab_window: root_window,
        event_mask: x::EventMask::BUTTON_PRESS
            | x::EventMask::BUTTON_RELEASE
            | x::EventMask::POINTER_MOTION,
        pointer_mode: x::GrabMode::Async,
        keyboard_mode: x::GrabMode::Async,
        confine_to: x::Window::none(),
        cursor: x::Cursor::none(),
        button: x::ButtonIndex::N1, // Left mouse button
        modifiers: modkey,
    });

    conn.send_request(&x::GrabButton {
        owner_events: true,
        grab_window: root_window,
        event_mask: x::EventMask::BUTTON_PRESS
            | x::EventMask::BUTTON_RELEASE
            | x::EventMask::POINTER_MOTION,
        pointer_mode: x::GrabMode::Async,
        keyboard_mode: x::GrabMode::Async,
        confine_to: x::Window::none(),
        cursor: x::Cursor::none(),
        button: x::ButtonIndex::N3, // Right mouse button
        modifiers: modkey,
    });

    conn.flush().unwrap();

    // window stack
    let mut windows: Vec<WindowData> = vec![];

    // store the last window Modkey + Click event to move the window around
    let mut start: Option<Box<x::ButtonPressEvent>> = None;
    let mut geom: Option<Box<x::GetGeometryReply>> = None;

    loop {
        let ev = conn.wait_for_event().unwrap();
        match ev {
            Event::X(x::Event::CreateNotify(ev)) => {
                let window_data = WindowData::new(ev.window());
                windows.push(window_data);

                windows = master_tiling(&conn, &screen, windows.clone());
            }

            Event::X(x::Event::DestroyNotify(ev)) => {
                let window = ev.window();
                windows.retain(|win| win.window != window);

                windows = master_tiling(&conn, &screen, windows.clone());
            }

            Event::X(x::Event::KeyPress(ev)) => {
                let window = ev.child();
                conn.send_request(&x::ConfigureWindow {
                    window,
                    value_list: &[x::ConfigWindow::StackMode(x::StackMode::Above)],
                });
            }

            Event::X(x::Event::ButtonPress(ev)) => {
                let window = ev.child();
                let cookie = conn.send_request(&x::GetGeometry {
                    drawable: x::Drawable::Window(window),
                });
                geom = Some(Box::new(conn.wait_for_reply(cookie).unwrap()));
                start = Some(Box::new(ev));
            }

            Event::X(x::Event::MotionNotify(ev)) => {
                let geom = geom.as_ref().unwrap();
                let mut window_x = geom.x() as i32;
                let mut window_y = geom.y() as i32;
                let mut window_w = geom.width() as u32;
                let mut window_h = geom.height() as u32;

                let start = start.as_ref().unwrap();
                let btn = start.detail();

                // pointer offset
                let pointer_x = ev.event_x() - start.root_x();
                let pointer_y = ev.event_y() - start.root_y();

                if btn == 1 {
                    // left mouse button, move window
                    window_x += pointer_x as i32;
                    window_y += pointer_y as i32;
                } else if btn == 3 {
                    // right mouse button, resize window
                    let w = cmp::max(1, window_w as i32 + pointer_x as i32);
                    let h = cmp::max(1, window_h as i32 + pointer_y as i32);
                    window_w = w as u32;
                    window_h = h as u32;
                }

                conn.send_request(&x::ConfigureWindow {
                    window: start.child(),
                    value_list: &[
                        x::ConfigWindow::X(window_x),
                        x::ConfigWindow::Y(window_y),
                        x::ConfigWindow::Width(window_w),
                        x::ConfigWindow::Height(window_h),
                    ],
                });
            }

            Event::X(x::Event::ButtonRelease(_)) => {
                start = None;
                geom = None;
            }

            _ => {}
        }

        conn.flush().unwrap();
    }
}
