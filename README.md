# `ancs-linux`
This is a Linux implementation of the Apple Notification Center Service (ANCS) protocol. It forwards notifications from an iOS device to a Linux desktop using Bluetooth Low Energy (BLE).

## Usage
1. Pair your iOS device with your Linux desktop.
   
   On most systems, your desktop will be visible to your iOS device as an audio sink (i.e. speaker), currently there is no built-in way to change this unless classic Bluetooth is disabled.
2. Obtain the fixed MAC address of your iOS device via utilities built into your DE or use `devices` command in the `bluetoothctl` shell.
3. After pairing, connect the iOS device to your desktop by clicking on the entry in your iOS device's settings menu. 
   
   **Initiating connection from the desktop might not work due to how classic Bluetooth and BLE work together. The application will connect to the device normally, but no notification can be received.**
4. Run `ancs-linux` with the MAC address of your iOS device as an argument. For example:
   ```sh
   ancs-linux AA:BB:CC:DD:EE:FF
   # To specify a different Bluetooth adapter
   ancs-linux --adapter hci1 AA:BB:CC:DD:EE:FF
   ```
5. Sit back and enjoy your notifications!
