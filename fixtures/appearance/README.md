# C# avatar appearance evidence

Captured from the local C# test grid at Tranquillity
`c70fee205f64ee8672a8d607cef6a733931da951`, using `cargo xtask live run login grid`. Pin6 (`1d7abb8337`) does not change `SendAppearance` or the avatar `ObjectUpdate` those bodies came from, so the bytes stay this capture.
The reference checkout was not modified. Raw captures stay under `.scratch/`.

The region sent one reliable zero-coded `ObjectUpdate` (high 12) and one reliable
zero-coded `AvatarAppearance` (low 158) after `AgentMovementComplete`. It did not
POST `getavatar`. Login stored the appearance with `setavatar` on the avatar
service: `AvatarHeight` 1.9 and the constructor visual-parameter list. The UDP
bodies are the oracle for what a viewer receives.

`avatar-appearance.body` is the captured message body, after the message number
and after zero-decoding. `AppearanceData` count is 0 (cofVersion -1, no baking).
There is one `AppearanceHover` of `(0, 0, 0)` and no `AttachmentBlock` count.
The texture entry is the 64-byte default bake (`GetBakesBytes` with no face
override). `visual-params.bin` is that body's visual-parameter list (218 bytes).

`object-update.body` is the same kind of body, with four folds:

- local id set to 0 (the capture's id is the scene allocator's)
- CRC set to 0 (the capture was already 0)
- packed position Z set to 0 (terrain height at the spawn; X and Y stay 128)
- `NameValue` replaced with the bytes `Redacted`

Before that replacement, `NameValue` was the public harness account with an
empty group title: `FirstName STRING RW SV Test`, `LastName STRING RW SV User`,
and `Title STRING RW SV ` plus a trailing NUL. The collision plane is
`(0, 0, 0, 1)`. Time dilation is `0xffff`. PCode is 47 and material is 4.
Scale is `(0.45, 0.6, 1.9)`. The packed look is `RotateToLookAt` of
`(0.99, 0.042, 0)` then `Quaternion.ToBytes`, and those 12 bytes are kept.

The sender id and the object full id are the harness account
`4f0c8d2e-7a51-4b8e-9c3d-2e6f1a0b7c55`, which is already named in the harness.
