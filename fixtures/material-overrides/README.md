# Pin8 negotiated material overrides

Two local test accounts received the [Q65b scene](../prim-updates/README.md). The home region reopens a disposable copy of that store, with `SupportViewerObjectsCache=false`, and patches `MatOvrd` only on that copy. The box root gains two override faces. The tree and the reflection-probe root gain one face each, with text that names the part, so a leaked send is visible. The committed scene is unchanged.

Only the control viewer asks the seed for `ModifyMaterialParams`. Pin8 then sets PBR support and, in `LLClientView.ProcessEntityUpdates`, sends one separate reliable `GenericStreamingMessage` (method 16757) on Task. The limited viewer asks for no PBR name and receives no such datagram. The tree is left out because its pcode is vegetation. The probe is left out because a reflection probe is not queued for this send. The HTTP `RenderMaterials` capability is not this capture.

[control.cap](control.cap) is that one datagram after the extractor checked the unredacted bytes re-encode exactly and replaced the local id with Q65b part 1. The notation is `{'id':i1,'te':[i0,i2],'od':[{'bc':[r1,r0,r0,r1]},{'ds':1}]}`. [limited.cap](limited.cap) is empty. Run 28576-2045190.

## Reproduce

From the repository root on Linux, run the capture under the shared live lock with a free port base and `UseSharedCompilation=false`. Keep `TMPDIR` and `CARGO_TARGET_DIR` under the worktree's `.scratch/`. The home region must use BasicPhysics.

```sh
cargo xtask live build
cargo xtask live run material-override grid --physics basicphysics
```

The command prints the scratch directory of the redacted captures. Compare them with this directory. Then:

```sh
cargo xtask live run material-override grid --region ferroan
```

That run sends the same patched store from Ferroan and compares reliability, zero-coding, method 16757 and the notation. `cargo test -p xtask grid::material_override` checks the committed datagrams offline.
