//! Data files compiled into the app (see `build.rs`): `ROUTES` are the checklists
//! in `data/routes/`, `LAYOUTS` the datamined layout graphs in `data/layouts/ggg/`,
//! each as (file name without `.json`, file text), sorted by file name.

include!(concat!(env!("OUT_DIR"), "/bundle.rs"));
