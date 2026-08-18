# DeviceOut

Route DAW bus audio to a Windows output device as a virtual microphone. | 把 DAW 总线音频路由到系统播放设备，当作虚拟麦克风。

VST3 插件，挂在轨道或总线上，把这段音频从 WASAPI 送到系统播放设备。

ASIO 状态下 DAW 只能走声卡物理口。搭配 [VB-Audio Virtual Cable](https://vb-audio.com/Cable/) 这类回环驱动，就可以在 ASIO
下直接把轨道输出当成 Windows 麦克风：插件输出选 `CABLE Input`，语音软件麦克风选 `CABLE Output`。自己有回环驱动也可以，不必非用
VB。
