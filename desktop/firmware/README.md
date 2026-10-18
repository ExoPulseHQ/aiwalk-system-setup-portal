# Security-key firmware

`pico-fido-waveshare-rp2350-zero.uf2` is what Team access writes to a new key board (`sealed.rs`, Set up the new key):
pico-fido, a FIDO2 security key for the RP2350, with the changes in `pico-keys-sdk.patch`. Both are AGPL-3.0; this
folder is where the source of the firmware we hand out lives.

| | |
|---|---|
| pico-fido | https://github.com/polhenarejos/pico-fido, tag v8.0 (6ffcb81191c7f4e44337e0eaecc788ba6db03b75) |
| pico-keys-sdk | https://github.com/polhenarejos/pico-keys-sdk, 7c5f4d9b95d5c17531a45c74c673822242d4d376 (the submodule at that tag) |
| pico-sdk | 2.1.1, with its tinyusb submodule |
| compiler | Arm GNU Toolchain 14.2.rel1 (arm-none-eabi) |
| board | `waveshare_rp2350_zero` (Waveshare RP2350-Zero: WS2812 light on GP16) |

What the patch changes, all in the light:

- Colour order RGB by default: this board's LED takes red first, and the usual GRB swapped red and green.
- The idle light breathes instead of blinking, at 12% of full brightness (`LED_WS2812_PEAK`), white while sealed notes
  are locked and green once they are unlocked. The smartcard interface's "card off" state (blue upstream, set after
  every look by the computer's card service) shows the same lock colour.
- Vendor command `CTAPHID 0xF0` (0x70 with the init bit), one byte from the computer: 1 unlocked (green 3 s, then
  green breathing), 0 an unlock failed (red blinks), 2 locked (red blinks, then white breathing). It only changes the
  light: any program on the computer can send it, so it proves nothing.

Build:

```
git clone --recursive -b v8.0 https://github.com/polhenarejos/pico-fido && cd pico-fido
git -C pico-keys-sdk apply <this folder>/pico-keys-sdk.patch
cmake -B build -DPICO_SDK_PATH=<pico-sdk 2.1.1> -DPICO_BOARD=waveshare_rp2350_zero -DPICO_PLATFORM=rp2350
cmake --build build -j    # build/pico_fido.uf2
```
