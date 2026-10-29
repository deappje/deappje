//! Generated protobuf types (buffa, under `proto`) and Connect service stubs
//! (under `connect`), mounted from the build's output tree one module pair
//! per proto package. Edit `proto/**/*.proto`, never these modules. The
//! scaffolder adds a nanoservice's package between the markers.

#![allow(missing_docs, clippy::all)]

pub mod proto {
    pub mod health {
        pub mod v1 {
            include!("../generated/health_v1/buffa/health.v1.rs");
        }
    }
    pub mod basable {
        pub mod config {
            pub mod v1 {
                include!("../generated/basable_config_v1/buffa/basable.config.v1.rs");
            }
        }
    }
    // basable:proto-modules-begin
    pub mod group {
        pub mod v1 {
            include!("../generated/group_v1/buffa/group.v1.rs");
        }
    }
    pub mod device {
        pub mod v1 {
            include!("../generated/device_v1/buffa/device.v1.rs");
        }
    }
    pub mod relay {
        pub mod v1 {
            include!("../generated/relay_v1/buffa/relay.v1.rs");
        }
    }
    pub mod storage {
        pub mod v1 {
            include!("../generated/storage_v1/buffa/storage.v1.rs");
        }
    }
    // basable:proto-modules-end
}

pub mod connect {
    pub mod health {
        pub mod v1 {
            include!("../generated/health_v1/connect/health.v1.rs");
        }
    }
    // basable:connect-modules-begin
    pub mod group {
        pub mod v1 {
            include!("../generated/group_v1/connect/group.v1.rs");
        }
    }
    pub mod device {
        pub mod v1 {
            include!("../generated/device_v1/connect/device.v1.rs");
        }
    }
    pub mod relay {
        pub mod v1 {
            include!("../generated/relay_v1/connect/relay.v1.rs");
        }
    }
    pub mod storage {
        pub mod v1 {
            include!("../generated/storage_v1/connect/storage.v1.rs");
        }
    }
    // basable:connect-modules-end
}
