// 引入所需的库
use bluest::{Adapter, AdvertisingDevice}; // bluest 库用于蓝牙交互
use chrono::Local; // chrono 库用于获取本地时间戳
use futures_lite::stream::StreamExt; // 用于异步流处理
use rosc::{OscMessage, OscPacket, OscType}; // rosc 库用于创建和编码 OSC 消息
use std::error::Error; // 标准错误处理
use std::net::{SocketAddr, UdpSocket}; // 用于发送 UDP 数据包 (OSC)
use std::str::FromStr; // 用于将字符串转换为 SocketAddr

// 定义常量，提高代码可读性和可维护性
const TARGET_COMPANY_ID: u16 = 0x0157; // Polar Electro Oy 的公司 ID，常用于心率监测器
const OSC_TARGET_ADDRESS: &str = "127.0.0.1:9000"; // OSC 目标 IP 地址和端口
// const OSC_TARGET_ADDRESS: &str = "192.168.9.101:9000"; // OSC 目标 IP 地址和端口
const OSC_HEART_RATE_PATH: &str = "/avatar/parameters/hr_percent"; // OSC 消息地址路径 (心率) - 确保这是接收浮点值的正确路径
const OSC_CONNECTION_STATUS_PATH: &str = "/avatar/parameters/hr_connected"; // OSC 消息地址路径 (连接状态)
const HEART_RATE_DATA_INDEX: usize = 3; // 假设心率数据在制造商数据的第4个字节 (索引3)
const HEART_RATE_FLOAT_MULTIPLIER: f32 = 0.005_f32; // 心率转换为浮点值的乘数

/// 异步主函数，程序入口点
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // --- 1. 蓝牙适配器初始化 ---
    println!(
        "[{}] 正在初始化蓝牙适配器...",
        Local::now().format("%H:%M:%S")
    );
    let adapter = Adapter::default()
        .await
        .ok_or("错误：未能找到蓝牙适配器。请确保蓝牙已开启且硬件可用。")?;
    adapter.wait_available().await?;
    println!(
        "[{}] 蓝牙适配器已成功初始化并可用。",
        Local::now().format("%H:%M:%S")
    );

    // --- 2. 解析 OSC 目标地址 ---
    let target_osc_addr = SocketAddr::from_str(OSC_TARGET_ADDRESS)
        .map_err(|e| format!("错误：无效的 OSC 目标地址 '{}': {}", OSC_TARGET_ADDRESS, e))?;
    println!(
        "[{}] OSC 目标地址: {}",
        Local::now().format("%H:%M:%S"),
        target_osc_addr
    );

    // --- 3. 创建 UDP socket ---
    let socket = UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| format!("错误：无法绑定 UDP socket 用于 OSC 发送: {}", e))?;
    println!(
        "[{}] UDP socket 已创建，准备发送 OSC 消息至 {}",
        Local::now().format("%H:%M:%S"),
        target_osc_addr
    );

    // --- 4. 开始扫描蓝牙设备 ---
    println!(
        "[{}] 准备开始扫描蓝牙设备...",
        Local::now().format("%H:%M:%S")
    );
    let mut scan = adapter.scan(&[]).await?;
    println!(
        "[{}] 扫描已启动。正在监听附近的蓝牙设备...",
        Local::now().format("%H:%M:%S")
    );

    // 用于存储第一个目标设备的名称
    let mut first_target_device_name: Option<String> = None;

    let mut first_hr_osc_logged = false;
    let mut first_conn_status_osc_logged = false;

    // --- 5. 处理发现的设备 ---
    while let Some(discovered_device) = scan.next().await {
        process_discovered_device(
            discovered_device,
            &mut first_target_device_name,
            &socket,
            target_osc_addr, // 传递已解析的 SocketAddr
            &mut first_hr_osc_logged,
            &mut first_conn_status_osc_logged,
        );
    }

    Ok(())
}

/// 处理单个发现的蓝牙设备
fn process_discovered_device(
    discovered_device: AdvertisingDevice,
    first_target_name_opt: &mut Option<String>,
    osc_socket: &UdpSocket,
    target_osc_addr: SocketAddr, // 接收已解析的 SocketAddr
    first_hr_osc_logged: &mut bool,
    first_conn_status_osc_logged: &mut bool,
) {
    if let Some(manufacturer_data) = &discovered_device.adv_data.manufacturer_data {
        if manufacturer_data.company_id == TARGET_COMPANY_ID {
            let current_device_name = discovered_device.device.name().unwrap_or_else(|err| {
                eprintln!(
                    "[{}] 注意：获取设备名称失败: {:?}，将使用默认名称。",
                    Local::now().format("%H:%M:%S"),
                    err
                );
                String::from("(未知设备)")
            });

            let mut process_this_device = false;
            match first_target_name_opt {
                Some(recorded_name) => {
                    if &current_device_name == recorded_name {
                        process_this_device = true;
                    }
                }
                None => {
                    // 首次发现目标制造商的设备，记录其名称
                    println!(
                        "[{}] 首次发现并记录目标设备名称: \"{}\"",
                        Local::now().format("%H:%M:%S"),
                        current_device_name
                    );
                    *first_target_name_opt = Some(current_device_name.clone());
                    process_this_device = true;
                }
            }

            if process_this_device {
                let rssi = discovered_device.rssi.unwrap_or_default();
                let current_timestamp_str = Local::now().format("%H:%M:%S").to_string();

                // 检查制造商数据是否足够长以包含心率数据
                if manufacturer_data.data.len() > HEART_RATE_DATA_INDEX {
                    let heart_rate_u8 = manufacturer_data.data[HEART_RATE_DATA_INDEX];
                    let heart_rate_float = heart_rate_u8 as f32 * HEART_RATE_FLOAT_MULTIPLIER;

                    println!(
                        "[{}] 设备: \"{}\", RSSI: {} dBm, 心率 (u8): {}, 心率 (float): {:.3}",
                        current_timestamp_str,
                        current_device_name, // 使用已获取的 current_device_name
                        rssi,
                        heart_rate_u8,
                        heart_rate_float // 打印转换后的浮点值
                    );

                    // 发送心率 OSC 消息 (作为浮点数)
                    send_osc_message(
                        osc_socket,
                        target_osc_addr,
                        OSC_HEART_RATE_PATH,
                        vec![OscType::Float(heart_rate_float)], // 发送浮点类型
                        "心率 (float)",
                        format!("{:.3}", heart_rate_float), // 日志记录浮点值
                        OSC_HEART_RATE_PATH,
                        first_hr_osc_logged,
                    );

                    // 发送连接状态 OSC 消息 (true)
                    // 注意：当前逻辑下，只有在心率数据有效时才会发送连接状态为 true。
                    // 并且，没有实现设备断开连接后发送 false 的逻辑。
                    send_osc_message(
                        osc_socket,
                        target_osc_addr,
                        OSC_CONNECTION_STATUS_PATH,
                        vec![OscType::Bool(true)],
                        "连接状态",
                        "true".to_string(),
                        OSC_CONNECTION_STATUS_PATH,
                        first_conn_status_osc_logged,
                    );
                } else {
                    // 制造商数据存在，但长度不足以提取心率
                    println!(
                        "[{}] 设备: \"{}\", RSSI: {} dBm. 制造商数据存在但长度不足以提取心率 (长度: {}，期望至少 {}).",
                        current_timestamp_str,
                        current_device_name,
                        rssi,
                        manufacturer_data.data.len(),
                        HEART_RATE_DATA_INDEX + 1
                    );
                    // 可选：即使没有心率数据，如果设备被识别，也可能希望发送连接状态。
                    // 当前代码不这样做，连接状态的发送依赖于有效的心率数据长度。
                }
            }
        }
    }
}

/// 构建并发送 OSC 消息的辅助函数
///
/// # Arguments
/// * `socket` - 用于发送 OSC 消息的 `UdpSocket`。
/// * `target_addr` - 已解析的 OSC 目标 `SocketAddr`。
/// * `path` - OSC 消息的地址路径。
/// * `args` - OSC 消息的参数列表。
/// * `message_type_for_log` - 用于日志记录的消息类型描述 (例如, "心率", "连接状态")。
/// * `value_for_log` - 用于日志记录的消息值 (例如, "0.375", "true")。
/// * `osc_path_for_log` - 用于日志记录的 OSC 路径。
/// * `first_log_sent` - 指向一个布尔标志的可变引用，指示是否已发送过此类型消息的第一个日志。
fn send_osc_message(
    socket: &UdpSocket,
    target_addr: SocketAddr,
    path: &str,
    args: Vec<OscType>,
    message_type_for_log: &str,
    value_for_log: String,
    osc_path_for_log: &str,
    first_log_sent: &mut bool,
) {
    let msg = OscMessage {
        addr: path.to_string(),
        args,
    };

    match rosc::encoder::encode(&OscPacket::Message(msg)) {
        Ok(encoded_packet) => {
            if let Err(e) = socket.send_to(&encoded_packet, target_addr) {
                eprintln!(
                    "[{}] 发送 OSC {} 消息到 {} 失败: {}",
                    Local::now().format("%H:%M:%S"),
                    message_type_for_log,
                    target_addr,
                    e
                );
            } else {
                if !*first_log_sent {
                    println!(
                        "[{}] {} {} 已通过 OSC 发送至 {} (路径: {})",
                        Local::now().format("%H:%M:%S"),
                        message_type_for_log,
                        value_for_log,
                        target_addr,
                        osc_path_for_log
                    );
                    *first_log_sent = true; // 标记此类型消息的第一个日志已发送
                }
            }
        }
        Err(e) => {
            eprintln!(
                "[{}] OSC {} 消息编码失败: {}",
                Local::now().format("%H:%M:%S"),
                message_type_for_log,
                e
            );
        }
    }
}
