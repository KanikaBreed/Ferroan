// Copyright (c) Linden Research, Inc.
// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later
//
// The appended block declarations below come from
//   scripts/messages/message_template.msg
//   secondlife/viewer @ ff75ab037607d8fe305f43f8b1f43126693ba2db
// Originally LGPL 2.1, per doc/LICENSE-source.txt at that revision.
// This validates only those additions; libopenmetaverse remains authoritative
// for every message header and every preceding block.

using System.Text.Json.Nodes;

namespace Ferroan.ReferenceOracle;

internal static partial class PacketCases
{
    const string ViewerRevision = "ff75ab037607d8fe305f43f8b1f43126693ba2db";
    static List<TemplateMessage> extendedTemplate;

    // Message headers here are placeholders. Only these suffix blocks are used.
    const string Suffixes = """
        version 2.0
        { AlertMessage Low 0 Trusted Unencoded
          { AgentInfo Variable { AgentID LLUUID } } }
        { AvatarAppearance Low 0 Trusted Unencoded
          { AttachmentBlock Variable { ID LLUUID } { AttachmentPoint U8 } } }
        { ChildAgentUpdate Low 0 Trusted Unencoded
          { AgentInventoryHost Variable { InventoryHost Variable 1 } } }
        { ImprovedInstantMessage Low 0 Trusted Unencoded
          { EstateBlock Single { EstateID U32 } }
          { MetaData Variable { Data Variable 2 } } }
        { InviteGroupResponse Low 0 Trusted Unencoded
          { GroupData Single { GroupLimit S32 } } }
        { RegionInfo Low 0 Trusted Unencoded
          { RegionInfo5 Variable
            { ChatWhisperRange F32 } { ChatNormalRange F32 } { ChatShoutRange F32 }
            { ChatWhisperOffset F32 } { ChatNormalOffset F32 } { ChatShoutOffset F32 }
            { ChatFlags U32 } }
          { CombatSettings Variable
            { CombatFlags U32 } { OnDeath U8 } { DamageThrottle F32 }
            { RegenerationRate F32 } { InvulnerabilyTime F32 } { DamageLimit F32 } } }
        { SimStatus Low 0 Trusted Unencoded
          { SimFlags Single { Flags U64 } } }
        { TransferInventory Low 0 Trusted Unencoded
          { ValidationBlock Single { NeedsValidation BOOL } { EstateID U32 } } }
        """;

    static string BlockShape(TemplateBlock block) =>
        block.Name + " " + block.Kind + " " + block.Count + " "
        + string.Join(";", block.Fields.Select(field => field.Name + " " + field.Type + " " + field.Size));

    static void ValidateExtensions(List<TemplateMessage> reference, List<TemplateMessage> committed)
    {
        var suffixes = Parse(Suffixes).ToDictionary(message => message.Name, StringComparer.Ordinal);
        if (reference.Count != committed.Count)
            throw new InvalidOperationException("the extended template changed the message count");
        for (int i = 0; i < reference.Count; i++)
        {
            TemplateMessage original = reference[i];
            TemplateMessage actual = committed[i];
            if (original.Header != actual.Header)
                throw new InvalidOperationException("the extended template changed a libopenmetaverse header: " + original.Name);
            var expected = original.Blocks.AsEnumerable();
            if (suffixes.TryGetValue(original.Name, out TemplateMessage extra))
                expected = expected.Concat(extra.Blocks);
            if (!expected.Select(BlockShape).SequenceEqual(actual.Blocks.Select(BlockShape)))
                throw new InvalidOperationException("the extended template has an unapproved block change: " + original.Name);
        }
    }

    static void CheckExtensionValidation()
    {
        const string original = "version 2.0 { AlertMessage Low 134 Trusted Unencoded { AlertData Single { Message Variable 1 } } }";
        const string valid = "version 2.0 { AlertMessage Low 134 Trusted Unencoded { AlertData Single { Message Variable 1 } } { AgentInfo Variable { AgentID LLUUID } } }";
        var reference = Parse(original);
        ValidateExtensions(reference, Parse(valid));
        foreach (string invalid in new[]
        {
            original,
            valid.Replace("Low 134", "Low 135"),
            valid.Replace("Trusted", "NotTrusted"),
            valid.Replace("Unencoded", "Zerocoded"),
            valid.Replace("Message Variable 1", "Message Variable 2"),
            valid.Replace("AgentID LLUUID", "AgentID U32"),
            valid.Replace("AgentInfo Variable", "AgentInfo Single"),
        })
        {
            bool rejected = false;
            try { ValidateExtensions(reference, Parse(invalid)); }
            catch (InvalidOperationException) { rejected = true; }
            if (!rejected)
                throw new InvalidOperationException("extension validator accepted an unapproved template mutation");
        }
    }

    static void AddExtensions(List<Fixture> list)
    {
        foreach (TemplateMessage suffix in Parse(Suffixes))
        {
            TemplateMessage original = template.Single(message => message.Name == suffix.Name);
            TemplateMessage extended = extendedTemplate.Single(message => message.Name == suffix.Name);
            byte[] prefix = EncodeRaw(original);
            byte[] raw = EncodeRaw(extended);
            if (!raw.AsSpan(0, prefix.Length).SequenceEqual(prefix))
                throw new InvalidOperationException("extended fixture changed libopenmetaverse raw prefix: " + suffix.Name);
            byte[] wire = MaybeZero(raw);
            bool hasClass = TryClass(original) != null;
            var parms = new JsonObject
            {
                ["name"] = suffix.Name,
                ["frequency"] = original.Frequency,
                ["number"] = (int)original.Number,
                ["has_class"] = hasClass,
                ["viewer"] = ViewerRevision,
                ["viewer_source"] = "scripts/messages/message_template.msg",
                ["libopenmetaverse_comparison"] = "raw-prefix-before-zerocoding",
                ["libopenmetaverse_prefix_bytes"] = prefix.Length,
                ["extension_source"] = "template-generated; not a captured C# sender",
                ["appended_blocks"] = new JsonArray(suffix.Blocks.Select(block => JsonValue.Create(block.Name)).ToArray()),
            };
            list.Add(new Fixture
            {
                Id = "packet-extension/" + suffix.Name,
                Family = "packet-extension",
                Format = "datagram",
                File = "packet-extension/" + suffix.Name + ".bin",
                Payload = wire,
                Params = parms,
                Decoded = Codec.Decode("packet-extension", "datagram", wire, parms),
                RoundTrip = Codec.RoundTrips("packet-extension", "datagram", wire, parms),
            });
        }
        Console.WriteLine("packet extensions: 8 viewer layouts match the libopenmetaverse raw prefixes; original class comparisons retained.");
    }
}
