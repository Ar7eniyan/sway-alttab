use std::{
    io::Seek,
    os::{
        fd::{AsFd, BorrowedFd},
        unix::prelude::FileExt,
    },
};

use wayland_client::{
    delegate_noop,
    globals::GlobalListContents,
    protocol::{
        wl_buffer, wl_compositor, wl_display, wl_registry, wl_shm, wl_shm_pool, wl_surface,
    },
    Connection, Dispatch, EventQueue, QueueHandle,
};

use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

fn allocate_shm(size: u64) -> std::fs::File {
    static SHM_FILENAME: &std::ffi::CStr = unsafe {
        // safety: the following literal MUST be null-terminated and not contain any interior null bytes
        std::ffi::CStr::from_bytes_with_nul_unchecked(b"wayland_surface_buffer\0")
    };

    let file: std::fs::File =
        nix::sys::memfd::memfd_create(SHM_FILENAME, nix::sys::memfd::MemFdCreateFlag::empty())
            .expect("can't create the anonymous file")
            .into();

    file.set_len(size)
        .expect(format!("can't resize the anonymous file to {size} bytes").as_str());
    file
}

struct WaylandState {
    surface: wl_surface::WlSurface,
    layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    shm_pool: wl_shm_pool::WlShmPool,
    surface_buffer: wl_buffer::WlBuffer,
    surface_buffer_file: std::fs::File,
    queue_handle: QueueHandle<Self>,
}

pub struct Gui {
    window: WaylandState,
    event_queue: EventQueue<WaylandState>,
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: <zwlr_layer_surface_v1::ZwlrLayerSurfaceV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                println!("Received wlr_layer_surface_v1::configure: serial = {serial}, size = {width}x{height}");
                layer_surface.ack_configure(serial);

                state.surface_buffer_file.seek(std::io::SeekFrom::Start(0)).unwrap();
                draw(&mut state.surface_buffer_file, (Gui::WIDTH as u32, Gui::HEIGHT as u32), 100);
                let buf = state.shm_pool.create_buffer(0, Gui::WIDTH, Gui::HEIGHT, Gui::STRIDE, wl_shm::Format::Argb8888, &state.queue_handle, ());
                state.surface.attach(Some(&buf), 0, 0);
                state.surface.damage(0, 0, Gui::WIDTH, Gui::HEIGHT);
                layer_surface.set_size(Gui::WIDTH as u32, Gui::HEIGHT as u32);
                state.surface.commit();
            }
            zwlr_layer_surface_v1::Event::Closed => {
                println!("Closing!");
                std::process::exit(0);
            }
            _ => {}
        }
    }
}

impl Gui {
    const WIDTH: i32 = 512;
    const HEIGHT: i32 = 512;
    const STRIDE: i32 = Self::WIDTH * 4;
    const SIZE: i32 = (Self::STRIDE * Self::HEIGHT) as _;

    pub fn new() -> Self {
        let conn = Connection::connect_to_env().expect("can't connect to Wayland socket");
        let display = conn.display();
        let (global_list, mut event_queue) =
            wayland_client::globals::registry_queue_init::<WaylandState>(&conn).unwrap();
        let qh = event_queue.handle();
        // let _registry = display.get_registry(qh, ());
        global_list.contents().with_list(|globals| {
            println!("Got globals:");
            for global in globals.iter() {
                println!("{:?}", global);
            }
        });

        let mut file = allocate_shm(Self::SIZE as u64);

        let shm: wl_shm::WlShm = global_list.bind(&qh, 1..=1, ()).unwrap();
        let compositor: wl_compositor::WlCompositor = global_list.bind(&qh, 1..=6, ()).unwrap();
        let layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1 =
            global_list.bind(&qh, 1..=4, ()).unwrap();

        let shm_pool = shm.create_pool(file.as_fd(), Self::SIZE as _, &qh, ());

        let surface_buffer = shm_pool.create_buffer(
            0,
            Self::WIDTH,
            Self::HEIGHT,
            Self::STRIDE,
            wl_shm::Format::Argb8888,
            &qh,
            (),
        );

        draw(&mut file, (Self::WIDTH as u32, Self::HEIGHT as u32), 100);

        let surface = compositor.create_surface(&qh, ());
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            None,
            zwlr_layer_shell_v1::Layer::Overlay,
            "test_namespace".to_string(),
            &qh,
            (),
        );
        surface.commit();

        let mut window = WaylandState {
                surface,
                shm_pool,
                layer_surface,
                surface_buffer,
                surface_buffer_file: file,
                queue_handle: qh,
            };

        event_queue.roundtrip(&mut window).unwrap();

        return Self {
            window,
            event_queue,
        };
    }

    pub fn run(&mut self) {
        println!("Running...");

        let mut t = 0.0;

        loop {
            self.redraw(t as u32);
            self.event_queue
                .blocking_dispatch(&mut self.window)
                .unwrap();
            t += 3.0;
            std::thread::sleep(std::time::Duration::new(0, 10000000));
        }
    }

    pub fn redraw(&mut self, t: u32) {
        self.window.surface_buffer_file.seek(std::io::SeekFrom::Start(0)).unwrap();
        draw(&mut self.window.surface_buffer_file, (Self::WIDTH as u32, Self::HEIGHT as u32), t);
        let buf = self.window.shm_pool.create_buffer(0, Self::WIDTH, Self::HEIGHT, Self::STRIDE, wl_shm::Format::Argb8888, &self.event_queue.handle(), ());
        self.window.surface.attach(Some(&buf), 0, 0);
        self.window.surface.damage(0, 0, Self::WIDTH, Self::HEIGHT);
        self.window.surface.commit();
    }
}

fn draw(tmp: &mut std::fs::File, (buf_x, buf_y): (u32, u32), mut t: u32) {
    use std::{cmp::min, io::Write};
    let mut buf = std::io::BufWriter::new(tmp);
    t = t % 0xff;
    for y in 0..buf_y {
        for x in 0..buf_x {
            let r = t * min(((buf_x - x) * 0xFF) / (buf_x), ((buf_y - y) * 0xFF) / (buf_y)) / 0xff;
            let g = t * min((x * 0xFF) / (buf_x), ((buf_y - y) * 0xFF) / (buf_y)) / 0xff;
            let b = t * min(((buf_x - x) * 0xFF) / (buf_x), (y * 0xFF) / (buf_y)) / 0xff;

            let color = (t << 24) + ((r & 0xFF) << 16) + ((g & 0xFF) << 8) + (b & 0xFF);
            buf.write_all(&color.to_ne_bytes()).unwrap();
        }
    }
    buf.flush().unwrap();
}

impl wayland_client::Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandState {
    fn event(
        _: &mut WaylandState,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &wayland_client::QueueHandle<WaylandState>,
    ) {
        println!("dynamic registry event: {event:?}")
    }
}

delegate_noop!(WaylandState: ignore wl_compositor::WlCompositor);
delegate_noop!(WaylandState: ignore wl_surface::WlSurface);
delegate_noop!(WaylandState: ignore wl_shm::WlShm);
delegate_noop!(WaylandState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(WaylandState: ignore wl_buffer::WlBuffer);
delegate_noop!(WaylandState: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
