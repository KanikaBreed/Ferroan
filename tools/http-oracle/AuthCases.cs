// Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

using System.Runtime.CompilerServices;
using System.Text;
using System.Text.Json;
using Nini.Config;
using OpenMetaverse;
using OpenSim.Data;
using OpenSim.Services.Interfaces;
using OpenSim.Services.AuthenticationService;
using OpenSim.Server.Handlers.Authentication;
using OpenSim.Server.Handlers.Authorization;

namespace Ferroan.HttpOracle;

static class AuthCases
{
    sealed class Permit : IAuthorizationService
    {
        public bool IsAuthorizedForRegion(string id, string first, string last, string region, out string message)
        {
            message = "";
            return true;
        }
    }

    sealed class Authorization() : AuthorizationServerPostHandler(new Permit())
    {
        public string Post(string xml)
        {
            using MemoryStream stream = new(Encoding.UTF8.GetBytes(xml));
            return Encoding.UTF8.GetString(ProcessRequest("/authorization", stream, null, null));
        }
    }

    sealed class Plain(IAuthenticationService service, IConfig config)
        : AuthenticationServerPostHandler(service, config, null)
    {
        public string Post(string body)
        {
            using MemoryStream stream = new(Encoding.UTF8.GetBytes(body));
            return Encoding.UTF8.GetString(ProcessRequest("/auth/plain", stream, null, null));
        }
    }

    // Supply the real service with an isolated data plugin without starting Robust.
    sealed class Webkey() : WebkeyAuthenticationService(null)
    {
        public static Webkey WithStore(IAuthenticationData store)
        {
            Webkey service = (Webkey)RuntimeHelpers.GetUninitializedObject(typeof(Webkey));
            service.m_Database = store;
            return service;
        }
    }

    sealed class MemoryStore : IAuthenticationData
    {
        public readonly UUID Key = UUID.Random();
        public string AccountType = "initial";
        public bool StoreResult = true;
        public int ClearAttempts;
        public AuthenticationData LastWrite;
        public AuthenticationData Get(UUID id) => new() { PrincipalID = id, Data = new()
        {
            ["webLoginKey"] = Key.ToString(), ["accountType"] = AccountType,
            ["passwordHash"] = "", ["passwordSalt"] = ""
        }};
        public bool Store(AuthenticationData data)
        {
            if (Equals(data.Data["webLoginKey"], UUID.Zero.ToString())) ClearAttempts++;
            if (StoreResult) { AccountType = (string)data.Data["accountType"]; LastWrite = data; }
            return StoreResult;
        }
        public bool SetToken(UUID id, string token, int lifetime) => true;
        public bool CheckToken(UUID id, string token, int lifetime) => throw new NotSupportedException();
        public bool SetDataItem(UUID id, string item, string value) => throw new NotSupportedException();
    }

    public static void Run(string directory, string revision, bool generate)
    {
        Authorization handler = new();
        List<object> xml = [];
        string[] children = [
            "<Extension><Nested><ID>hidden</ID></Nested></Extension><ID>direct</ID>",
            "<!--before--><ID>a<!--inside-->b</ID><!--after-->",
            "<ID>a<![CDATA[<&>]]>b</ID>", "<![CDATA[unused]]><ID>direct</ID>",
            "<ID>first</ID><ID>second</ID>", "<ID/><ID>second</ID>",
            "<ID xmlns='urn:other'>foreign</ID><ID>direct</ID>", "<ID>x&#x20;y</ID>",
            "<ID><nested/></ID>", "<FirstName><nested/></FirstName>", "<Unknown></Mismatch>",
            "<ID>  </ID>", "<ID> a </ID>", "<ID><![CDATA[  ]]></ID>",
            "<ID xml:space='preserve'>  </ID>", "<ID>  <![CDATA[a]]>  </ID>",
            "<ID>first</ID><ID><ignored/></ID>", "<ID> a <!--c--> b </ID>"
        ];
        const string xsi = "http://www.w3.org/2001/XMLSchema-instance";
        string WithAttributes(string attributes) =>
            $"<AuthorizationRequest xmlns:i='{xsi}' {attributes}><ID>root</ID></AuthorizationRequest>";
        string[] roots = [
            "<?xml version='1.1'?><AuthorizationRequest/>",
            WithAttributes("i:type='Other'"),
            WithAttributes("xmlns:t='urn:other' i:type='t:AuthorizationRequest'"),
            WithAttributes("i:nil='bogus'"), WithAttributes("i:nil='true'"), WithAttributes("i:nil='1'"),
            WithAttributes("i:type='AuthorizationRequest' i:nil='false'"),
            WithAttributes("i:nil=' 0 '"), WithAttributes("xmlns:u='urn:other' u:nil='true'"),
            WithAttributes("i:type=' AuthorizationRequest '"),
            "<AuthorizationRequest><ID xmlns:i='" + xsi + "' i:nil='true'>child</ID></AuthorizationRequest>"
        ];
        foreach (string request in children.Select(content => "<AuthorizationRequest>" + content + "</AuthorizationRequest>").Concat(roots))
        {
            try { xml.Add(new { request, status = 200, body = handler.Post(request) }); }
            // BaseHttpServer maps a deserialization failure or a null request to 500.
            catch (Exception e) when (e is InvalidOperationException or NullReferenceException)
            {
                xml.Add(new { request, status = 500, body = "" });
            }
        }
        UUID id = new("11111111-1111-4111-8111-111111111111");
        MemoryStore store = new();
        Webkey service = Webkey.WithStore(store);
        IConfig config = new IniConfigSource().AddConfig("AuthenticationService");
        config.Set("AllowSetAuthInfo", true);
        Plain plain = new(service, config);
        string requestForm = $"METHOD=setauthinfo&PRINCIPAL={id}&AccountType[]=";
        string body = plain.Post(requestForm);
        store.StoreResult = false;
        bool issued = service.Authenticate(id, store.Key.ToString(), 30).Length > 0;
        MemoryStore resetStore = new();
        bool reset = Webkey.WithStore(resetStore).SetPassword(id, "fixture");
        var write = resetStore.LastWrite.Data;
        var reset_fields = new { stored = reset, writes_previous_key = Equals(write["webLoginKey"], resetStore.Key.ToString()),
            account_type = write["accountType"], salt_length = ((string)write["passwordSalt"]).Length,
            hash_length = ((string)write["passwordHash"]).Length };
        var result = new { revision, xml, reset_fields, list = new { request = requestForm, body, account_type = store.AccountType },
            clear_refused = new { clear_attempts = store.ClearAttempts, token_issued = issued } };
        string json = JsonSerializer.Serialize(result, new JsonSerializerOptions { WriteIndented = true, NewLine = "\n" }) + "\n";
        string path = Path.Combine(directory, "cases.json");
        if (generate) { Directory.CreateDirectory(directory); File.WriteAllText(path, json); }
        else if (File.ReadAllText(path) != json) throw new InvalidOperationException("auth edge fixture differs");
    }
}
