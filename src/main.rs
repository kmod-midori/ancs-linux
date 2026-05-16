use std::{collections::HashMap, io::Cursor};

use ancs::{
    attributes::{
        app::AppAttributeID,
        command::CommandID,
        event::{EventFlag, EventID},
        notification::NotificationAttributeID,
        AppAttribute,
    },
    characteristics::{
        control_point::{GetAppAttributesRequest, GetNotificationAttributesRequest},
        data_source,
    },
};
use anyhow::{bail, Result};
use bluer::{
    gatt::remote::{Characteristic, CharacteristicWriteRequest},
    Adapter, Address, Uuid, UuidExt,
};
use byteorder_pack::UnpackFrom;
use clap::Parser;
use futures::{pin_mut, StreamExt as _};

struct AncsProcessor {
    control_point: Option<Characteristic>,
    app_names: HashMap<String, String>,
}

const ANCS_SERVICE_UUID: Uuid = Uuid::from_u128(0x7905F431B5CE4E99A40F4B1E122D00D0);

const HID_REPORT_MAP: &[u8] = &[
    0x05, 0x01, // Usage Page (Generic Desktop)
    0x09, 0x06, // Usage (Keyboard)
    0xa1, 0x01, // Collection (Application)
    0x05, 0x07, //   Usage Page (Keyboard/Keypad)
    0x19, 0xe0, //   Usage Minimum (224)
    0x29, 0xe7, //   Usage Maximum (231)
    0x15, 0x00, //   Logical Minimum (0)
    0x25, 0x01, //   Logical Maximum (1)
    0x75, 0x01, //   Report Size (1)
    0x95, 0x08, //   Report Count (8)
    0x81, 0x02, //   Input (Data, Variable, Absolute) — modifier keys
    0x95, 0x01, //   Report Count (1)
    0x75, 0x08, //   Report Size (8)
    0x81, 0x01, //   Input (Constant) — reserved byte
    0x95, 0x06, //   Report Count (6)
    0x75, 0x08, //   Report Size (8)
    0x15, 0x00, //   Logical Minimum (0)
    0x26, 0xff, 0x00, //   Logical Maximum (255)
    0x05, 0x07, //   Usage Page (Keyboard/Keypad)
    0x19, 0x00, //   Usage Minimum (0)
    0x2a, 0xff, 0x00, //   Usage Maximum (255)
    0x81, 0x00, //   Input (Data, Array) — key codes
    0xc0, // End Collection
];

async fn serve_hid_gatt(
    adapter: &Adapter,
) -> Result<(
    bluer::gatt::local::ApplicationHandle,
    bluer::adv::AdvertisementHandle,
)> {
    use bluer::adv::{Advertisement, Type};
    use bluer::gatt::local::{
        Application, Characteristic, CharacteristicNotify, CharacteristicNotifyMethod,
        CharacteristicRead, CharacteristicWrite, CharacteristicWriteMethod, Descriptor,
        DescriptorRead, Service,
    };

    let app = Application {
        services: vec![Service {
            uuid: Uuid::from_u16(0x1812),
            primary: true,
            characteristics: vec![
                // HID Information: version 1.11, not localized, NormallyConnectable
                Characteristic {
                    uuid: Uuid::from_u16(0x2A4A),
                    read: Some(CharacteristicRead {
                        read: true,
                        fun: Box::new(|req| {
                            Box::pin(async move {
                                log::info!(
                                    "HID Information read by {}",
                                    req.device_address
                                );
                                Ok(vec![0x11, 0x01, 0x00, 0x02])
                            })
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                // Report Map
                Characteristic {
                    uuid: Uuid::from_u16(0x2A4B),
                    read: Some(CharacteristicRead {
                        read: true,
                        fun: Box::new(|req| {
                            Box::pin(async move {
                                log::info!(
                                    "HID Report Map read by {}",
                                    req.device_address
                                );
                                Ok(HID_REPORT_MAP.to_vec())
                            })
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                // HID Control Point — accept Suspend/Exit Suspend, no-op
                Characteristic {
                    uuid: Uuid::from_u16(0x2A4C),
                    write: Some(CharacteristicWrite {
                        write: true,
                        method: CharacteristicWriteMethod::Fun(Box::new(|value, req| {
                            Box::pin(async move {
                                log::info!(
                                    "HID Control Point written by {}: {:?}",
                                    req.device_address,
                                    value
                                );
                                Ok(())
                            })
                        })),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                // Protocol Mode — report protocol, accept writes
                Characteristic {
                    uuid: Uuid::from_u16(0x2A4E),
                    read: Some(CharacteristicRead {
                        read: true,
                        fun: Box::new(|req| {
                            Box::pin(async move {
                                log::info!(
                                    "HID Protocol Mode read by {}",
                                    req.device_address
                                );
                                Ok(vec![0x01])
                            })
                        }),
                        ..Default::default()
                    }),
                    write: Some(CharacteristicWrite {
                        write: true,
                        method: CharacteristicWriteMethod::Fun(Box::new(|value, req| {
                            Box::pin(async move {
                                log::info!(
                                    "HID Protocol Mode written by {}: {:?}",
                                    req.device_address,
                                    value
                                );
                                Ok(())
                            })
                        })),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                // Report (input) with Report Reference descriptor
                Characteristic {
                    uuid: Uuid::from_u16(0x2A4D),
                    read: Some(CharacteristicRead {
                        read: true,
                        fun: Box::new(|req| {
                            Box::pin(async move {
                                log::info!(
                                    "HID Report read by {}",
                                    req.device_address
                                );
                                Ok(vec![0u8; 8])
                            })
                        }),
                        ..Default::default()
                    }),
                    notify: Some(CharacteristicNotify {
                        notify: true,
                        indicate: false,
                        method: CharacteristicNotifyMethod::Fun(Box::new(|notifier| {
                            Box::pin(async move {
                                log::info!("HID Report notifications started");
                                notifier.stopped().await;
                                log::info!("HID Report notifications stopped");
                            })
                        })),
                        ..Default::default()
                    }),
                    descriptors: vec![Descriptor {
                        uuid: Uuid::from_u16(0x2908),
                        read: Some(DescriptorRead {
                            read: true,
                            fun: Box::new(|req| {
                                Box::pin(async move {
                                    log::info!(
                                        "HID Report Reference read by {}",
                                        req.device_address
                                    );
                                    Ok(vec![0x00, 0x01])
                                })
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    log::info!("Registering HID GATT application");
    let app_handle = adapter.serve_gatt_application(app).await?;

    log::info!("Starting HID advertisement");
    let adv = Advertisement {
        advertisement_type: Type::Peripheral,
        service_uuids: [Uuid::from_u16(0x1812)].into(),
        appearance: Some(0x03C1),
        local_name: Some("ANCS HID Bridge".into()),
        discoverable: Some(true),
        ..Default::default()
    };
    let adv_handle = adapter.advertise(adv).await?;

    Ok((app_handle, adv_handle))
}

impl AncsProcessor {
    pub fn new() -> Self {
        Self {
            control_point: None,
            app_names: HashMap::new(),
        }
    }

    pub async fn main_loop(mut self, device_addr: Address, adapter: &Adapter) -> Result<()> {
        let device = adapter.device(device_addr)?;

        if !device.is_connected().await? {
            log::debug!("Device {} is not connected", device_addr);
            return Ok(());
        }

        log::info!("Device {} is connected", device_addr);

        let services = device.services().await?;
        let mut ancs_service = None;
        for s in services {
            if s.uuid().await? == ANCS_SERVICE_UUID {
                ancs_service = Some(s);
                break;
            }
        }

        let ancs_service = match ancs_service {
            Some(s) => s,
            None => {
                bail!("ANCS service not found");
            }
        };

        let mut notification_source = None;
        let mut data_source = None;
        let mut control_point = None;
        let noti_source_uuid: Uuid = "9FBF120D-6301-42D9-8C58-25E699A21DBD".parse()?;
        let data_source_uuid: Uuid = "22EAC6E9-24D6-4BB5-BE44-B36ACE7C7BFB".parse()?;
        let control_point_uuid: Uuid = "69D1D8F3-45E1-49A8-9821-9BBDFDAAD9D9".parse()?;
        for c in ancs_service.characteristics().await? {
            let uuid = c.uuid().await?;

            if uuid == noti_source_uuid {
                notification_source = Some(c);
            } else if uuid == data_source_uuid {
                data_source = Some(c);
            } else if uuid == control_point_uuid {
                control_point = Some(c);
            }
        }

        let notification_source = match notification_source {
            Some(c) => c,
            None => {
                bail!("Notification source not found");
            }
        };

        let data_source = match data_source {
            Some(c) => c,
            None => {
                bail!("Data source not found");
            }
        };

        let control_point = match control_point {
            Some(c) => c,
            None => {
                bail!("Control point not found");
            }
        };

        self.control_point = Some(control_point);

        let data_source_stream = data_source.notify().await?;
        pin_mut!(data_source_stream);

        let notification_stream = notification_source.notify().await?;
        pin_mut!(notification_stream);

        let events_stream = adapter.events().await?;
        pin_mut!(events_stream);

        log::info!("Starting to listen for notifications");

        loop {
            tokio::select! {
                Some(noti) = notification_stream.next() => {
                    self.process_notification(noti).await?;

                }
                Some(data) = data_source_stream.next() => {
                    self.process_data(data).await?;
                }
                Some(event) = events_stream.next() => {
                    if let bluer::AdapterEvent::DeviceRemoved(addr) = event {
                        if addr == device_addr {
                            log::info!("Device removed, stopping");
                            break;
                        }
                    }
                }
                else => break,
            }
        }

        Ok(())
    }

    async fn process_notification(&mut self, noti: Vec<u8>) -> Result<()> {
        let (event_id, event_flags, _category_id, _category_count, notification_uid) =
            <(u8, u8, u8, u8, u32)>::unpack_from_le(&mut Cursor::new(&noti))?;

        if event_id == EventID::NotificationRemoved as u8 {
            return Ok(());
        }

        if event_flags & EventFlag::PreExisting as u8 != 0 {
            return Ok(());
        }

        let cmd = GetNotificationAttributesRequest {
            command_id: CommandID::GetNotificationAttributes,
            notification_uid,
            attribute_ids: vec![
                (NotificationAttributeID::AppIdentifier, None),
                (NotificationAttributeID::Title, Some(64)),
                (NotificationAttributeID::Subtitle, Some(64)),
                (NotificationAttributeID::Message, Some(64)),
            ],
        };

        self.write_control_point(&Vec::from(cmd)).await?;

        Ok(())
    }

    async fn process_data(&mut self, data: Vec<u8>) -> Result<()> {
        match data[0] {
            0 => {
                let notif = match data_source::GetNotificationAttributesResponse::parse(&data) {
                    Ok((_, app)) => app,
                    Err(e) => {
                        bail!("Error parsing notification attributes: {:?}", e);
                    }
                };
                log::info!("Notif: {:?}", notif);

                let mut app_id_to_query = None;

                let mut desktop_notification = notify_rust::Notification::new();
                for attr in notif.attribute_list {
                    match attr.id {
                        NotificationAttributeID::AppIdentifier => {
                            if let Some(id) = attr.value {
                                if let Some(name) = self.app_names.get(&id) {
                                    desktop_notification.appname(name);
                                } else {
                                    // Query for app name
                                    desktop_notification.appname(&id);
                                    app_id_to_query = Some(id);
                                }
                            }
                        }
                        NotificationAttributeID::Title => {
                            if let Some(v) = attr.value {
                                desktop_notification.summary(&v);
                            }
                        }
                        NotificationAttributeID::Message => {
                            if let Some(v) = attr.value {
                                desktop_notification.body(&v);
                            }
                        }
                        _ => {}
                    }
                }

                let handle = desktop_notification.show_async().await?;
                log::info!(
                    "Shown notification {} with desktop handle {}",
                    notif.notification_uid,
                    handle.id()
                );

                if let Some(app_id) = app_id_to_query {
                    log::info!("Querying app name for {}", app_id);
                    let cmd = GetAppAttributesRequest {
                        command_id: CommandID::GetAppAttributes,
                        app_identifier: app_id,
                        attribute_ids: vec![AppAttributeID::DisplayName],
                    };
                    self.write_control_point(&Vec::from(cmd)).await?;
                }
            }
            1 => {
                let mut app_id = vec![];
                let mut offset = 1;
                for i in offset..data.len() {
                    offset += 1;
                    if data[i] == 0 {
                        break;
                    }
                    app_id.push(data[i]);
                }
                let app_id = String::from_utf8_lossy(&app_id); // NULL-terminated string

                let attribute = match AppAttribute::parse(&data[offset..]) {
                    Ok((_, attribute)) => attribute,
                    Err(e) => {
                        bail!("Error parsing app attributes: {:?}", e);
                    }
                };

                if attribute.id == AppAttributeID::DisplayName {
                    if let Some(name) = attribute.value {
                        log::info!("{} => {}", app_id, name);
                        // Store app name
                        self.app_names.insert(app_id.to_string(), name);
                    }
                } else {
                    log::info!("Unknown app attribute: {:?}", attribute);
                }
            }
            _ => {}
        }

        Ok(())
    }

    async fn write_control_point(&self, data: &[u8]) -> Result<()> {
        if let Some(control_point) = &self.control_point {
            control_point
                .write_ext(
                    data,
                    &CharacteristicWriteRequest {
                        op_type: bluer::gatt::WriteOp::Request,
                        ..Default::default()
                    },
                )
                .await?;
        }

        Ok(())
    }
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(
        help = "Public Bluetooth MAC address of the device to connect to (as shown in system or `bluetoothctl`)"
    )]
    device_addr: Address,

    #[arg(long, help = "Bluetooth adapter name to use, if not the default one")]
    adapter: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    let args = Args::parse();

    let session = bluer::Session::new().await?;
    let adapter = if let Some(name) = args.adapter {
        session.adapter(&name)?
    } else {
        session.default_adapter().await?
    };
    adapter.set_powered(true).await?;

    log::info!("Using adapter: {}", adapter.name());

    // Register a HID keyboard GATT server so iOS auto-reconnects.
    let (_hid_app, _hid_adv) = match serve_hid_gatt(&adapter).await {
        Ok(handles) => handles,
        Err(e) => {
            log::warn!("Failed to set up HID GATT service: {:?}", e);
            return Err(e);
        }
    };

    loop {
        let proc = AncsProcessor::new();
        if let Err(e) = proc.main_loop(args.device_addr, &adapter).await {
            log::error!("Error: {:?}", e);
        }
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
}
