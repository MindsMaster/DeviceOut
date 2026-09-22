![DeviceOut](docs/images/hero-en.png)

[![Email](https://img.shields.io/badge/Email-an5w1r%40163.com-blue.svg)](mailto:an5w1r@163.com)
[![Discord](https://img.shields.io/badge/Discord-5865F2.svg?logo=discord&logoColor=white)](https://discord.gg/u9n5E894wK)
[![QQ](https://img.shields.io/badge/QQ-1046048297-12B7F5.svg?logo=qq&logoColor=white)](https://qun.qq.com/universal-share/share?ac=1&authKey=QEsKQUh0Z2dhNgn2lQDrnRRGD0MALQvTfF3FPr5ZYOv8i/lKdNp8G6gjTgYrD/Rw&busi_data=eyJncm91cENvZGUiOiIxMDQ2MDQ4Mjk3IiwidG9rZW4iOiJRR3lINzRUTUt0U2M0bDRkeTRhTERCT0ZscFlsTUxBNm9TdjUwelYxZXN2dzdUWUlyWnY0Mi9oOHI3c2JBOEYrIiwidWluIjoiMTE3MzM0ODYxMCJ9&data=EuNu7PewjDdMH0GF2GuEOYyIXT8iT25RJajcGVD3qL5LXkiATxSyzB-gdxqgbbFkbRvkyzoiJQ_sJ8-tBfFbSuW4NYFdjzdIr5Z1E8reAiM&svctype=5&tempid=h5_group_info)

# DeviceOut

[English](README.md) | [中文](README.zh-CN.md)

Windows VST3. Push DAW bus audio to a system playback device.

When the interface is running ASIO, you may still want the host’s processed output in Discord, OBS, and similar apps.

Under ASIO the host opens the device exclusively. That bus never enters the Windows audio session, so other programs cannot monitor it from the system side. The workaround is to send this audio to a loopback playback device (usually `CABLE Input`) and have those apps take the matching capture device (`CABLE Output`).

## Requirements

- Windows 10 / 11, 64-bit
- A VST3 host (Studio One, REAPER, Cubase, and others)
- A loopback driver if you need a virtual mic. [VB-Audio Cable](https://vb-audio.com/Cable/) is recommended

## Install VB-Cable

1. Open [https://vb-audio.com/Cable/](https://vb-audio.com/Cable/)
2. Download the free **VB-CABLE Driver**
3. Unzip and run `VBCABLE_Setup_x64.exe` as administrator
4. Reboot if Windows asks

Windows then adds two devices:

- **CABLE Input** — playback
- **CABLE Output** — capture (the mic for voice apps)

Any other loopback driver works the same way. VB is not required.

## Install DeviceOut

Run the installer. Restart the host or rescan plugins.

## Usage

1. Insert DeviceOut on a track or the master
2. Set **OUTPUT DEVICE** to `CABLE Input`
3. In Discord / OBS / other voice apps, set the microphone to `CABLE Output`

![editor](docs/images/editor-en.png)
