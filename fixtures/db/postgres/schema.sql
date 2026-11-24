--
-- PostgreSQL database dump
--


-- Dumped from database version 16.16 (Debian 16.16-1.pgdg13+1)
-- Dumped by pg_dump version 16.16 (Debian 16.16-1.pgdg13+1)

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

SET default_tablespace = '';

SET default_table_access_method = heap;

--
-- Name: agentprefs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.agentprefs (
    "PrincipalID" uuid NOT NULL,
    "AccessPrefs" character(2) DEFAULT 'M'::bpchar NOT NULL,
    "HoverHeight" double precision DEFAULT 0 NOT NULL,
    "Language" character(5) DEFAULT 'en-us'::bpchar NOT NULL,
    "LanguageIsPublic" boolean DEFAULT true NOT NULL,
    "PermEveryone" integer DEFAULT 0 NOT NULL,
    "PermGroup" integer DEFAULT 0 NOT NULL,
    "PermNextOwner" integer DEFAULT 532480 NOT NULL
);


--
-- Name: agents; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.agents (
    "UUID" uuid NOT NULL,
    "sessionID" uuid NOT NULL,
    "secureSessionID" uuid NOT NULL,
    "agentIP" character varying(16) NOT NULL,
    "agentPort" integer NOT NULL,
    "agentOnline" smallint NOT NULL,
    "loginTime" integer NOT NULL,
    "logoutTime" integer NOT NULL,
    "currentRegion" uuid NOT NULL,
    "currentHandle" bigint NOT NULL,
    "currentPos" character varying(64) NOT NULL
);


--
-- Name: assets; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.assets (
    id uuid NOT NULL,
    name character varying(64) NOT NULL,
    description character varying(64) NOT NULL,
    "assetType" smallint NOT NULL,
    local boolean NOT NULL,
    temporary boolean NOT NULL,
    data bytea NOT NULL,
    create_time integer,
    access_time integer,
    asset_flags integer DEFAULT 0 NOT NULL,
    creatorid character varying(128) DEFAULT ''::character varying NOT NULL
);


--
-- Name: auth; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.auth (
    uuid uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "passwordHash" character varying(32) NOT NULL,
    "passwordSalt" character varying(32) NOT NULL,
    "webLoginKey" character varying(255) NOT NULL,
    "accountType" character varying(32) DEFAULT 'UserAccount'::character varying NOT NULL
);


--
-- Name: avatarappearance; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.avatarappearance (
    "Owner" uuid NOT NULL,
    "Serial" integer NOT NULL,
    "Visual_Params" bytea NOT NULL,
    "Texture" bytea NOT NULL,
    "Avatar_Height" double precision NOT NULL,
    "Body_Item" uuid NOT NULL,
    "Body_Asset" uuid NOT NULL,
    "Skin_Item" uuid NOT NULL,
    "Skin_Asset" uuid NOT NULL,
    "Hair_Item" uuid NOT NULL,
    "Hair_Asset" uuid NOT NULL,
    "Eyes_Item" uuid NOT NULL,
    "Eyes_Asset" uuid NOT NULL,
    "Shirt_Item" uuid NOT NULL,
    "Shirt_Asset" uuid NOT NULL,
    "Pants_Item" uuid NOT NULL,
    "Pants_Asset" uuid NOT NULL,
    "Shoes_Item" uuid NOT NULL,
    "Shoes_Asset" uuid NOT NULL,
    "Socks_Item" uuid NOT NULL,
    "Socks_Asset" uuid NOT NULL,
    "Jacket_Item" uuid NOT NULL,
    "Jacket_Asset" uuid NOT NULL,
    "Gloves_Item" uuid NOT NULL,
    "Gloves_Asset" uuid NOT NULL,
    "Undershirt_Item" uuid NOT NULL,
    "Undershirt_Asset" uuid NOT NULL,
    "Underpants_Item" uuid NOT NULL,
    "Underpants_Asset" uuid NOT NULL,
    "Skirt_Item" uuid NOT NULL,
    "Skirt_Asset" uuid NOT NULL
);


--
-- Name: avatarattachments; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.avatarattachments (
    "UUID" uuid NOT NULL,
    attachpoint integer NOT NULL,
    item uuid NOT NULL,
    asset uuid NOT NULL
);


--
-- Name: avatars; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.avatars (
    "PrincipalID" uuid NOT NULL,
    "Name" character varying(32) NOT NULL,
    "Value" text DEFAULT ''::text NOT NULL
);


--
-- Name: bakedterrain; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.bakedterrain (
    "RegionUUID" uuid,
    "Revision" integer,
    "Heightfield" bytea
);


--
-- Name: classifieds; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.classifieds (
    classifieduuid uuid NOT NULL,
    creatoruuid uuid NOT NULL,
    creationdate integer NOT NULL,
    expirationdate integer NOT NULL,
    category character varying(20) NOT NULL,
    name character varying(255) NOT NULL,
    description text NOT NULL,
    parceluuid uuid NOT NULL,
    parentestate integer NOT NULL,
    snapshotuuid uuid NOT NULL,
    simname character varying(255) NOT NULL,
    posglobal character varying(255) NOT NULL,
    parcelname character varying(255) NOT NULL,
    classifiedflags integer NOT NULL,
    priceforlisting integer NOT NULL
);


--
-- Name: estate_allowed_experiences; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.estate_allowed_experiences (
    "EstateID" integer NOT NULL,
    uuid uuid NOT NULL
);


--
-- Name: estate_blocked_experiences; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.estate_blocked_experiences (
    "EstateID" integer NOT NULL,
    uuid uuid NOT NULL
);


--
-- Name: estate_groups; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.estate_groups (
    "EstateID" integer NOT NULL,
    uuid uuid NOT NULL
);


--
-- Name: estate_key_experiences; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.estate_key_experiences (
    "EstateID" integer NOT NULL,
    uuid uuid NOT NULL
);


--
-- Name: estate_managers; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.estate_managers (
    "EstateID" integer NOT NULL,
    uuid uuid NOT NULL
);


--
-- Name: estate_map; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.estate_map (
    "RegionID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "EstateID" integer NOT NULL
);


--
-- Name: estate_settings_id; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.estate_settings_id
    START WITH 100
    INCREMENT BY 100
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: estate_settings; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.estate_settings (
    "EstateID" integer DEFAULT nextval('public.estate_settings_id'::regclass) NOT NULL,
    "EstateName" character varying(64) DEFAULT NULL::character varying,
    "AbuseEmailToEstateOwner" boolean NOT NULL,
    "DenyAnonymous" boolean NOT NULL,
    "ResetHomeOnTeleport" boolean NOT NULL,
    "FixedSun" boolean NOT NULL,
    "DenyTransacted" boolean NOT NULL,
    "BlockDwell" boolean NOT NULL,
    "DenyIdentified" boolean NOT NULL,
    "AllowVoice" boolean NOT NULL,
    "UseGlobalTime" boolean NOT NULL,
    "PricePerMeter" integer NOT NULL,
    "TaxFree" boolean NOT NULL,
    "AllowDirectTeleport" boolean NOT NULL,
    "RedirectGridX" integer NOT NULL,
    "RedirectGridY" integer NOT NULL,
    "ParentEstateID" integer NOT NULL,
    "SunPosition" double precision NOT NULL,
    "EstateSkipScripts" boolean NOT NULL,
    "BillableFactor" double precision NOT NULL,
    "PublicAccess" boolean NOT NULL,
    "AbuseEmail" character varying(255) NOT NULL,
    "EstateOwner" uuid NOT NULL,
    "DenyMinors" boolean NOT NULL,
    "AllowLandmark" boolean DEFAULT true NOT NULL,
    "AllowParcelChanges" boolean DEFAULT true NOT NULL,
    "AllowSetHome" boolean DEFAULT true NOT NULL,
    "AllowEnviromentOverride" boolean NOT NULL
);


--
-- Name: estate_users; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.estate_users (
    "EstateID" integer NOT NULL,
    uuid uuid NOT NULL
);


--
-- Name: estateban; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.estateban (
    "EstateID" integer NOT NULL,
    "bannedUUID" uuid NOT NULL,
    "bannedIp" character varying(16),
    "bannedIpHostMask" character varying(16),
    "bannedNameMask" character varying(64),
    "banningUUID" uuid NOT NULL,
    "banTime" integer DEFAULT 0 NOT NULL
);


--
-- Name: friends; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.friends (
    "PrincipalID" character varying(255) DEFAULT '00000000-0000-0000-0000-000000000000'::character varying NOT NULL,
    "Friend" character varying(255) NOT NULL,
    "Flags" character(16) DEFAULT '0'::bpchar NOT NULL,
    "Offered" character varying(32) DEFAULT 0 NOT NULL
);


--
-- Name: fsassets; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.fsassets (
    id uuid NOT NULL,
    type integer NOT NULL,
    hash character(64) NOT NULL,
    create_time integer DEFAULT 0 NOT NULL,
    access_time integer DEFAULT 0 NOT NULL,
    asset_flags integer DEFAULT 0 NOT NULL
);


--
-- Name: griduser; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.griduser (
    "UserID" character varying(255) NOT NULL,
    "HomeRegionID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "HomePosition" character(64) DEFAULT '<0,0,0>'::bpchar NOT NULL,
    "HomeLookAt" character(64) DEFAULT '<0,0,0>'::bpchar NOT NULL,
    "LastRegionID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "LastPosition" character(64) DEFAULT '<0,0,0>'::bpchar NOT NULL,
    "LastLookAt" character(64) DEFAULT '<0,0,0>'::bpchar NOT NULL,
    "Online" character(5) DEFAULT 'false'::bpchar NOT NULL,
    "Login" character(16) DEFAULT '0'::bpchar NOT NULL,
    "Logout" character(16) DEFAULT '0'::bpchar NOT NULL
);


--
-- Name: hg_traveling_data; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.hg_traveling_data (
    "SessionID" character varying(36) NOT NULL,
    "UserID" character varying(36) NOT NULL,
    "GridExternalName" character varying(255) DEFAULT ''::character varying NOT NULL,
    "ServiceToken" character varying(255) DEFAULT ''::character varying NOT NULL,
    "ClientIPAddress" character varying(16) DEFAULT ''::character varying NOT NULL,
    "MyIPAddress" character varying(16) DEFAULT ''::character varying NOT NULL,
    "TMStamp" timestamp without time zone DEFAULT now() NOT NULL
);


--
-- Name: im_offiline_id; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.im_offiline_id
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: im_offline; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.im_offline (
    "ID" integer NOT NULL,
    "PrincipalID" uuid NOT NULL,
    "Message" text NOT NULL,
    "TMStamp" timestamp(6) without time zone DEFAULT clock_timestamp() NOT NULL,
    "FromID" uuid NOT NULL
);


--
-- Name: im_offline_ID_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public."im_offline_ID_seq"
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: im_offline_ID_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public."im_offline_ID_seq" OWNED BY public.im_offline."ID";


--
-- Name: inventoryfolders; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.inventoryfolders (
    "folderID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "agentID" uuid,
    "parentFolderID" uuid,
    "folderName" character varying(64) DEFAULT NULL::character varying,
    type smallint DEFAULT 0 NOT NULL,
    version integer DEFAULT 0 NOT NULL
);


--
-- Name: inventoryitems; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.inventoryitems (
    "inventoryID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "assetID" uuid,
    "assetType" integer,
    "parentFolderID" uuid,
    "avatarID" uuid,
    "inventoryName" character varying(64) DEFAULT NULL::character varying,
    "inventoryDescription" character varying(128) DEFAULT NULL::character varying,
    "inventoryNextPermissions" integer,
    "inventoryCurrentPermissions" integer,
    "invType" integer,
    "creatorID" character varying(255) DEFAULT '00000000-0000-0000-0000-000000000000'::uuid,
    "inventoryBasePermissions" integer DEFAULT 0 NOT NULL,
    "inventoryEveryOnePermissions" integer DEFAULT 0 NOT NULL,
    "salePrice" integer,
    "saleType" smallint,
    "creationDate" integer,
    "groupID" uuid,
    "groupOwned" boolean,
    flags integer,
    "inventoryGroupPermissions" integer DEFAULT 0 NOT NULL,
    "experienceID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL
);


--
-- Name: land; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.land (
    "UUID" uuid NOT NULL,
    "RegionUUID" uuid,
    "LocalLandID" integer,
    "Bitmap" bytea,
    "Name" character varying(255),
    "Description" character varying(255),
    "OwnerUUID" uuid,
    "IsGroupOwned" boolean,
    "Area" integer,
    "AuctionID" integer,
    "Category" integer,
    "ClaimDate" integer,
    "ClaimPrice" integer,
    "GroupUUID" uuid,
    "SalePrice" integer,
    "LandStatus" integer,
    "LandFlags" bigint,
    "LandingType" integer,
    "MediaAutoScale" integer,
    "MediaTextureUUID" uuid,
    "MediaURL" character varying(255),
    "MusicURL" character varying(255),
    "PassHours" double precision,
    "PassPrice" integer,
    "SnapshotUUID" uuid,
    "UserLocationX" double precision,
    "UserLocationY" double precision,
    "UserLocationZ" double precision,
    "UserLookAtX" double precision,
    "UserLookAtY" double precision,
    "UserLookAtZ" double precision,
    "AuthbuyerID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "OtherCleanTime" integer DEFAULT 0 NOT NULL,
    "Dwell" integer DEFAULT 0 NOT NULL,
    "MediaType" character varying(32) DEFAULT 'none/none'::character varying NOT NULL,
    "MediaDescription" character varying(255) DEFAULT ''::character varying NOT NULL,
    "MediaSize" character varying(16) DEFAULT '0,0'::character varying NOT NULL,
    "MediaLoop" boolean DEFAULT false NOT NULL,
    "ObscureMusic" boolean DEFAULT false NOT NULL,
    "ObscureMedia" boolean DEFAULT false NOT NULL,
    "SeeAVs" smallint DEFAULT 1 NOT NULL,
    "AnyAVSounds" smallint DEFAULT 1 NOT NULL,
    "GroupAVSounds" smallint DEFAULT 1 NOT NULL,
    environment character varying
);


--
-- Name: landaccesslist; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.landaccesslist (
    "LandUUID" uuid,
    "AccessUUID" uuid,
    "Flags" integer,
    "Expires" integer DEFAULT 0 NOT NULL
);


--
-- Name: logs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.logs (
    "logID" integer NOT NULL,
    target character varying(36) DEFAULT NULL::character varying,
    server character varying(64) DEFAULT NULL::character varying,
    method character varying(64) DEFAULT NULL::character varying,
    arguments character varying(255) DEFAULT NULL::character varying,
    priority integer,
    message text
);


--
-- Name: migrations; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.migrations (
    name character varying(100),
    version integer
);


--
-- Name: mutelist; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.mutelist (
    "AgentID" character varying(36) NOT NULL,
    "MuteID" character varying(36) DEFAULT '00000000-0000-0000-0000-000000000000'::character varying NOT NULL,
    "MuteName" character varying(64) DEFAULT ''::character varying NOT NULL,
    "MuteType" integer DEFAULT 1 NOT NULL,
    "MuteFlags" integer DEFAULT 0 NOT NULL,
    "Stamp" integer NOT NULL
);


--
-- Name: os_groups_bans; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.os_groups_bans (
    "GroupID" uuid NOT NULL,
    "PrincipalID" character varying(255) DEFAULT ''::character varying NOT NULL,
    "BanTime" integer DEFAULT 0 NOT NULL
);


--
-- Name: os_groups_groups; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.os_groups_groups (
    "GroupID" uuid NOT NULL,
    "Location" character varying(255) DEFAULT ''::character varying NOT NULL,
    "Name" character varying(255) DEFAULT ''::character varying NOT NULL,
    "Charter" text NOT NULL,
    "InsigniaID" uuid NOT NULL,
    "FounderID" uuid NOT NULL,
    "MembershipFee" integer DEFAULT 0 NOT NULL,
    "OpenEnrollment" character varying(255) DEFAULT ''::character varying NOT NULL,
    "ShowInList" integer DEFAULT 0 NOT NULL,
    "AllowPublish" integer DEFAULT 0 NOT NULL,
    "MaturePublish" integer DEFAULT 0 NOT NULL,
    "OwnerRoleID" uuid NOT NULL
);


--
-- Name: os_groups_invites; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.os_groups_invites (
    "InviteID" uuid NOT NULL,
    "GroupID" uuid NOT NULL,
    "RoleID" uuid NOT NULL,
    "PrincipalID" character varying(255) DEFAULT ''::character varying NOT NULL,
    "TMStamp" timestamp without time zone DEFAULT now() NOT NULL
);


--
-- Name: os_groups_membership; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.os_groups_membership (
    "GroupID" uuid NOT NULL,
    "PrincipalID" character varying(255) DEFAULT ''::character varying NOT NULL,
    "SelectedRoleID" uuid NOT NULL,
    "Contribution" integer DEFAULT 0 NOT NULL,
    "ListInProfile" integer DEFAULT 1 NOT NULL,
    "AcceptNotices" integer DEFAULT 1 NOT NULL,
    "AccessToken" uuid NOT NULL
);


--
-- Name: os_groups_notices; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.os_groups_notices (
    "GroupID" uuid NOT NULL,
    "NoticeID" uuid NOT NULL,
    "TMStamp" integer DEFAULT 0 NOT NULL,
    "FromName" character varying(255) DEFAULT ''::character varying NOT NULL,
    "Subject" character varying(255) DEFAULT ''::character varying NOT NULL,
    "Message" text NOT NULL,
    "HasAttachment" integer DEFAULT 0 NOT NULL,
    "AttachmentType" integer DEFAULT 0 NOT NULL,
    "AttachmentName" character varying(128) DEFAULT ''::character varying NOT NULL,
    "AttachmentItemID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "AttachmentOwnerID" character varying(255) DEFAULT ''::character varying NOT NULL
);


--
-- Name: os_groups_principals; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.os_groups_principals (
    "PrincipalID" character varying(255) DEFAULT ''::character varying NOT NULL,
    "ActiveGroupID" uuid NOT NULL
);


--
-- Name: os_groups_rolemembership; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.os_groups_rolemembership (
    "GroupID" uuid NOT NULL,
    "RoleID" uuid NOT NULL,
    "PrincipalID" character varying(255) DEFAULT ''::character varying NOT NULL
);


--
-- Name: os_groups_roles; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.os_groups_roles (
    "GroupID" uuid NOT NULL,
    "RoleID" uuid NOT NULL,
    "Name" character varying(255) DEFAULT ''::character varying NOT NULL,
    "Description" character varying(255) DEFAULT ''::character varying NOT NULL,
    "Title" character varying(255) DEFAULT ''::character varying NOT NULL,
    "Powers" character varying(36) DEFAULT ''::character varying NOT NULL
);


--
-- Name: presence; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.presence (
    "UserID" character varying(255) NOT NULL,
    "RegionID" uuid NOT NULL,
    "SessionID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "SecureSessionID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "LastSeen" timestamp without time zone
);


--
-- Name: primitems; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.primitems (
    "itemID" uuid NOT NULL,
    "primID" uuid,
    "assetID" uuid,
    "parentFolderID" uuid,
    "invType" integer,
    "assetType" integer,
    name character varying(255),
    description character varying(255),
    "creationDate" character varying(255),
    "creatorIDOld" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "ownerID" uuid,
    "lastOwnerID" uuid,
    "groupID" uuid,
    "nextPermissions" integer,
    "currentPermissions" integer,
    "basePermissions" integer,
    "everyonePermissions" integer,
    "groupPermissions" integer,
    flags integer DEFAULT 0 NOT NULL,
    "creatorID" character varying(255) DEFAULT '00000000-0000-0000-0000-000000000000'::character varying NOT NULL,
    "experienceID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL
);


--
-- Name: prims; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.prims (
    "UUID" uuid NOT NULL,
    "RegionUUID" uuid,
    "ParentID" integer,
    "CreationDate" integer,
    "Name" character varying(255),
    "SceneGroupID" uuid,
    "Text" character varying(255),
    "Description" character varying(255),
    "SitName" character varying(255),
    "TouchName" character varying(255),
    "ObjectFlags" integer,
    "CreatorIDOld" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "OwnerID" uuid,
    "GroupID" uuid,
    "LastOwnerID" uuid,
    "OwnerMask" integer,
    "NextOwnerMask" integer,
    "GroupMask" integer,
    "EveryoneMask" integer,
    "BaseMask" integer,
    "PositionX" double precision,
    "PositionY" double precision,
    "PositionZ" double precision,
    "GroupPositionX" double precision,
    "GroupPositionY" double precision,
    "GroupPositionZ" double precision,
    "VelocityX" double precision,
    "VelocityY" double precision,
    "VelocityZ" double precision,
    "AngularVelocityX" double precision,
    "AngularVelocityY" double precision,
    "AngularVelocityZ" double precision,
    "AccelerationX" double precision,
    "AccelerationY" double precision,
    "AccelerationZ" double precision,
    "RotationX" double precision,
    "RotationY" double precision,
    "RotationZ" double precision,
    "RotationW" double precision,
    "SitTargetOffsetX" double precision,
    "SitTargetOffsetY" double precision,
    "SitTargetOffsetZ" double precision,
    "SitTargetOrientW" double precision,
    "SitTargetOrientX" double precision,
    "SitTargetOrientY" double precision,
    "SitTargetOrientZ" double precision,
    "PayPrice" integer DEFAULT 0 NOT NULL,
    "PayButton1" integer DEFAULT 0 NOT NULL,
    "PayButton2" integer DEFAULT 0 NOT NULL,
    "PayButton3" integer DEFAULT 0 NOT NULL,
    "PayButton4" integer DEFAULT 0 NOT NULL,
    "LoopedSound" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "LoopedSoundGain" double precision DEFAULT 0.0 NOT NULL,
    "TextureAnimation" bytea,
    "OmegaX" double precision DEFAULT 0.0 NOT NULL,
    "OmegaY" double precision DEFAULT 0.0 NOT NULL,
    "OmegaZ" double precision DEFAULT 0.0 NOT NULL,
    "CameraEyeOffsetX" double precision DEFAULT 0.0 NOT NULL,
    "CameraEyeOffsetY" double precision DEFAULT 0.0 NOT NULL,
    "CameraEyeOffsetZ" double precision DEFAULT 0.0 NOT NULL,
    "CameraAtOffsetX" double precision DEFAULT 0.0 NOT NULL,
    "CameraAtOffsetY" double precision DEFAULT 0.0 NOT NULL,
    "CameraAtOffsetZ" double precision DEFAULT 0.0 NOT NULL,
    "ForceMouselook" smallint DEFAULT 0 NOT NULL,
    "ScriptAccessPin" integer DEFAULT 0 NOT NULL,
    "AllowedDrop" smallint DEFAULT 0 NOT NULL,
    "DieAtEdge" smallint DEFAULT 0 NOT NULL,
    "SalePrice" integer DEFAULT 10 NOT NULL,
    "SaleType" smallint DEFAULT 0 NOT NULL,
    "ColorR" integer DEFAULT 0 NOT NULL,
    "ColorG" integer DEFAULT 0 NOT NULL,
    "ColorB" integer DEFAULT 0 NOT NULL,
    "ColorA" integer DEFAULT 0 NOT NULL,
    "ParticleSystem" bytea,
    "ClickAction" smallint DEFAULT 0 NOT NULL,
    "Material" smallint DEFAULT 3 NOT NULL,
    "CollisionSound" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "CollisionSoundVolume" double precision DEFAULT 0.0 NOT NULL,
    "LinkNumber" integer DEFAULT 0 NOT NULL,
    "PassTouches" boolean DEFAULT false NOT NULL,
    "MediaURL" character varying(255),
    "CreatorID" character varying(255) DEFAULT '00000000-0000-0000-0000-000000000000'::character varying NOT NULL,
    "DynAttrs" text,
    "PhysicsShapeType" smallint DEFAULT '0'::smallint NOT NULL,
    "Density" double precision DEFAULT '1000'::double precision NOT NULL,
    "GravityModifier" double precision DEFAULT '1'::double precision NOT NULL,
    "Friction" double precision DEFAULT '0.6'::double precision NOT NULL,
    "Restitution" double precision DEFAULT '0.5'::double precision NOT NULL,
    "PassCollisions" boolean NOT NULL,
    "Vehicle" text,
    "RotationAxisLocks" smallint DEFAULT 0 NOT NULL,
    "RezzerID" uuid,
    "PhysInertia" text,
    standtargetx real DEFAULT '0'::real,
    standtargety real DEFAULT '0'::real,
    standtargetz real DEFAULT '0'::real,
    sitactrange real DEFAULT '0'::real,
    pseudocrc integer DEFAULT 0,
    sopanims bytea,
    linksetdata character varying,
    "StartStr" text,
    "SitTargetActive" smallint
);


--
-- Name: primshapes; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.primshapes (
    "UUID" uuid NOT NULL,
    "Shape" integer,
    "ScaleX" double precision,
    "ScaleY" double precision,
    "ScaleZ" double precision,
    "PCode" integer,
    "PathBegin" integer,
    "PathEnd" integer,
    "PathScaleX" integer,
    "PathScaleY" integer,
    "PathShearX" integer,
    "PathShearY" integer,
    "PathSkew" integer,
    "PathCurve" integer,
    "PathRadiusOffset" integer,
    "PathRevolutions" integer,
    "PathTaperX" integer,
    "PathTaperY" integer,
    "PathTwist" integer,
    "PathTwistBegin" integer,
    "ProfileBegin" integer,
    "ProfileEnd" integer,
    "ProfileCurve" integer,
    "ProfileHollow" integer,
    "State" integer,
    "Texture" bytea,
    "ExtraParams" bytea,
    "Media" text,
    "MatOvrd" bytea
);


--
-- Name: regionban; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.regionban (
    "regionUUID" uuid NOT NULL,
    "bannedUUID" uuid NOT NULL,
    "bannedIp" character varying(16) NOT NULL,
    "bannedIpHostMask" character varying(16) NOT NULL
);


--
-- Name: regionenvironment; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.regionenvironment (
    region_id uuid NOT NULL,
    llsd_settings character varying NOT NULL
);


--
-- Name: regionextra; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.regionextra (
    "RegionID" character(36) NOT NULL,
    "Name" character varying(32) NOT NULL,
    value text
);


--
-- Name: regions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.regions (
    uuid uuid NOT NULL,
    "regionHandle" bigint,
    "regionName" character varying(128) NOT NULL,
    "regionRecvKey" character varying(128),
    "regionSendKey" character varying(128),
    "regionSecret" character varying(128),
    "regionDataURI" character varying(128),
    "serverIP" character varying(64),
    "serverPort" integer,
    "serverURI" character varying(255),
    "locX" integer,
    "locY" integer,
    "locZ" integer,
    "eastOverrideHandle" bigint,
    "westOverrideHandle" bigint,
    "southOverrideHandle" bigint,
    "northOverrideHandle" bigint,
    "regionAssetURI" character varying(255),
    "regionAssetRecvKey" character varying(128),
    "regionAssetSendKey" character varying(128),
    "regionUserURI" character varying(255),
    "regionUserRecvKey" character varying(128),
    "regionUserSendKey" character varying(128),
    "regionMapTexture" uuid,
    "serverHttpPort" integer,
    "serverRemotingPort" integer,
    owner_uuid uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "originUUID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    access integer DEFAULT 0,
    "ScopeID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid,
    "sizeX" integer DEFAULT 0 NOT NULL,
    "sizeY" integer DEFAULT 0 NOT NULL,
    flags integer DEFAULT 0 NOT NULL,
    last_seen integer DEFAULT 0 NOT NULL,
    "PrincipalID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "Token" character varying(255) DEFAULT 0 NOT NULL,
    "parcelMapTexture" uuid,
    "viewerLocalAddress" character varying(64)
);


--
-- Name: regionsettings; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.regionsettings (
    "regionUUID" uuid NOT NULL,
    block_terraform boolean NOT NULL,
    block_fly boolean NOT NULL,
    allow_damage boolean NOT NULL,
    restrict_pushing boolean NOT NULL,
    allow_land_resell boolean NOT NULL,
    allow_land_join_divide boolean NOT NULL,
    block_show_in_search boolean NOT NULL,
    agent_limit integer NOT NULL,
    object_bonus double precision NOT NULL,
    maturity integer NOT NULL,
    disable_scripts boolean NOT NULL,
    disable_collisions boolean NOT NULL,
    disable_physics boolean NOT NULL,
    terrain_texture_1 uuid NOT NULL,
    terrain_texture_2 uuid NOT NULL,
    terrain_texture_3 uuid NOT NULL,
    terrain_texture_4 uuid NOT NULL,
    elevation_1_nw double precision NOT NULL,
    elevation_2_nw double precision NOT NULL,
    elevation_1_ne double precision NOT NULL,
    elevation_2_ne double precision NOT NULL,
    elevation_1_se double precision NOT NULL,
    elevation_2_se double precision NOT NULL,
    elevation_1_sw double precision NOT NULL,
    elevation_2_sw double precision NOT NULL,
    water_height double precision NOT NULL,
    terrain_raise_limit double precision NOT NULL,
    terrain_lower_limit double precision NOT NULL,
    use_estate_sun boolean NOT NULL,
    fixed_sun boolean NOT NULL,
    sun_position double precision NOT NULL,
    covenant uuid,
    "Sandbox" boolean NOT NULL,
    sunvectorx double precision DEFAULT 0 NOT NULL,
    sunvectory double precision DEFAULT 0 NOT NULL,
    sunvectorz double precision DEFAULT 0 NOT NULL,
    loaded_creation_id character varying(64),
    loaded_creation_datetime integer DEFAULT 0 NOT NULL,
    "map_tile_ID" character(36) DEFAULT '00000000-0000-0000-0000-000000000000'::bpchar NOT NULL,
    covenant_datetime integer DEFAULT 0 NOT NULL,
    "TelehubObject" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "parcel_tile_ID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    block_search boolean NOT NULL,
    casino boolean NOT NULL,
    "cacheID" uuid,
    "TerrainPBR1" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "TerrainPBR2" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "TerrainPBR3" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "TerrainPBR4" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL
);


--
-- Name: regionwindlight; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.regionwindlight (
    region_id character varying(36) DEFAULT '000000-0000-0000-0000-000000000000'::character varying NOT NULL,
    water_color_r double precision DEFAULT '4'::double precision NOT NULL,
    water_color_g double precision DEFAULT '38'::double precision NOT NULL,
    water_color_b double precision DEFAULT '64'::double precision NOT NULL,
    water_fog_density_exponent double precision DEFAULT '4'::double precision NOT NULL,
    underwater_fog_modifier double precision DEFAULT '0.25'::double precision NOT NULL,
    reflection_wavelet_scale_1 double precision DEFAULT '2'::double precision NOT NULL,
    reflection_wavelet_scale_2 double precision DEFAULT '2'::double precision NOT NULL,
    reflection_wavelet_scale_3 double precision DEFAULT '2'::double precision NOT NULL,
    fresnel_scale double precision DEFAULT '0.4'::double precision NOT NULL,
    fresnel_offset double precision DEFAULT '0.5'::double precision NOT NULL,
    refract_scale_above double precision DEFAULT '0.03'::double precision NOT NULL,
    refract_scale_below double precision DEFAULT '0.2'::double precision NOT NULL,
    blur_multiplier double precision DEFAULT '0.04'::double precision NOT NULL,
    big_wave_direction_x double precision DEFAULT '1.05'::double precision NOT NULL,
    big_wave_direction_y double precision DEFAULT '-0.42'::double precision NOT NULL,
    little_wave_direction_x double precision DEFAULT '1.11'::double precision NOT NULL,
    little_wave_direction_y double precision DEFAULT '-1.16'::double precision NOT NULL,
    normal_map_texture character varying(36) DEFAULT '822ded49-9a6c-f61c-cb89-6df54f42cdf4'::character varying NOT NULL,
    horizon_r double precision DEFAULT '0.25'::double precision NOT NULL,
    horizon_g double precision DEFAULT '0.25'::double precision NOT NULL,
    horizon_b double precision DEFAULT '0.32'::double precision NOT NULL,
    horizon_i double precision DEFAULT '0.32'::double precision NOT NULL,
    haze_horizon double precision DEFAULT '0.19'::double precision NOT NULL,
    blue_density_r double precision DEFAULT '0.12'::double precision NOT NULL,
    blue_density_g double precision DEFAULT '0.22'::double precision NOT NULL,
    blue_density_b double precision DEFAULT '0.38'::double precision NOT NULL,
    blue_density_i double precision DEFAULT '0.38'::double precision NOT NULL,
    haze_density double precision DEFAULT '0.7'::double precision NOT NULL,
    density_multiplier double precision DEFAULT '0.18'::double precision NOT NULL,
    distance_multiplier double precision DEFAULT '0.8'::double precision NOT NULL,
    max_altitude integer DEFAULT 1605 NOT NULL,
    sun_moon_color_r double precision DEFAULT '0.24'::double precision NOT NULL,
    sun_moon_color_g double precision DEFAULT '0.26'::double precision NOT NULL,
    sun_moon_color_b double precision DEFAULT '0.3'::double precision NOT NULL,
    sun_moon_color_i double precision DEFAULT '0.3'::double precision NOT NULL,
    sun_moon_position double precision DEFAULT '0.317'::double precision NOT NULL,
    ambient_r double precision DEFAULT '0.35'::double precision NOT NULL,
    ambient_g double precision DEFAULT '0.35'::double precision NOT NULL,
    ambient_b double precision DEFAULT '0.35'::double precision NOT NULL,
    ambient_i double precision DEFAULT '0.35'::double precision NOT NULL,
    east_angle double precision DEFAULT '0'::double precision NOT NULL,
    sun_glow_focus double precision DEFAULT '0.1'::double precision NOT NULL,
    sun_glow_size double precision DEFAULT '1.75'::double precision NOT NULL,
    scene_gamma double precision DEFAULT '1'::double precision NOT NULL,
    star_brightness double precision DEFAULT '0'::double precision NOT NULL,
    cloud_color_r double precision DEFAULT '0.41'::double precision NOT NULL,
    cloud_color_g double precision DEFAULT '0.41'::double precision NOT NULL,
    cloud_color_b double precision DEFAULT '0.41'::double precision NOT NULL,
    cloud_color_i double precision DEFAULT '0.41'::double precision NOT NULL,
    cloud_x double precision DEFAULT '1'::double precision NOT NULL,
    cloud_y double precision DEFAULT '0.53'::double precision NOT NULL,
    cloud_density double precision DEFAULT '1'::double precision NOT NULL,
    cloud_coverage double precision DEFAULT '0.27'::double precision NOT NULL,
    cloud_scale double precision DEFAULT '0.42'::double precision NOT NULL,
    cloud_detail_x double precision DEFAULT '1'::double precision NOT NULL,
    cloud_detail_y double precision DEFAULT '0.53'::double precision NOT NULL,
    cloud_detail_density double precision DEFAULT '0.12'::double precision NOT NULL,
    cloud_scroll_x double precision DEFAULT '0.2'::double precision NOT NULL,
    cloud_scroll_x_lock boolean DEFAULT false NOT NULL,
    cloud_scroll_y double precision DEFAULT '0.01'::double precision NOT NULL,
    cloud_scroll_y_lock boolean DEFAULT false NOT NULL,
    draw_classic_clouds boolean DEFAULT false NOT NULL
);


--
-- Name: spawn_points; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.spawn_points (
    "RegionUUID" uuid NOT NULL,
    "Yaw" double precision NOT NULL,
    "Pitch" double precision NOT NULL,
    "Distance" double precision NOT NULL
);


--
-- Name: terrain; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.terrain (
    "RegionUUID" uuid,
    "Revision" integer,
    "Heightfield" bytea
);


--
-- Name: tokens; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.tokens (
    uuid uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    token character varying(255) NOT NULL,
    validity timestamp without time zone NOT NULL
);


--
-- Name: useraccounts; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.useraccounts (
    "PrincipalID" uuid NOT NULL,
    "ScopeID" uuid NOT NULL,
    "FirstName" character varying(64) NOT NULL,
    "LastName" character varying(64) NOT NULL,
    "Email" character varying(64),
    "ServiceURLs" text,
    "Created" integer,
    "UserLevel" integer DEFAULT 0 NOT NULL,
    "UserFlags" integer DEFAULT 0 NOT NULL,
    "UserTitle" character varying(64) DEFAULT ''::character varying NOT NULL,
    active integer DEFAULT 1 NOT NULL,
    "DisplayName" character varying(31),
    "NameChanged" integer DEFAULT 0,
    "UserCountry" character varying(100) DEFAULT ''::character varying NOT NULL
);


--
-- Name: userdata; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.userdata (
    "UserId" uuid NOT NULL,
    "TagId" character varying(64) NOT NULL,
    "DataKey" character varying(255),
    "DataVal" character varying(255)
);


--
-- Name: userfriends; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.userfriends (
    "ownerID" uuid NOT NULL,
    "friendID" uuid NOT NULL,
    "friendPerms" integer NOT NULL,
    datetimestamp integer NOT NULL
);


--
-- Name: usernotes; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.usernotes (
    useruuid uuid NOT NULL,
    targetuuid uuid NOT NULL,
    notes text NOT NULL
);


--
-- Name: userpicks; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.userpicks (
    pickuuid uuid NOT NULL,
    creatoruuid uuid NOT NULL,
    toppick boolean NOT NULL,
    parceluuid uuid NOT NULL,
    name character varying(255) NOT NULL,
    description text NOT NULL,
    snapshotuuid character varying(36) NOT NULL,
    "user" character varying(255) NOT NULL,
    originalname character varying(255) NOT NULL,
    simname character varying(255) NOT NULL,
    posglobal character varying(255) NOT NULL,
    sortorder integer NOT NULL,
    enabled boolean NOT NULL,
    gatekeeper character varying(255)
);


--
-- Name: userprofile; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.userprofile (
    useruuid uuid NOT NULL,
    "profilePartner" uuid NOT NULL,
    "profileAllowPublish" boolean NOT NULL,
    "profileMaturePublish" boolean NOT NULL,
    "profileURL" character varying(255) NOT NULL,
    "profileWantToMask" integer NOT NULL,
    "profileWantToText" text NOT NULL,
    "profileSkillsMask" integer NOT NULL,
    "profileSkillsText" text NOT NULL,
    "profileLanguages" text NOT NULL,
    "profileImage" uuid NOT NULL,
    "profileAboutText" text NOT NULL,
    "profileFirstImage" uuid NOT NULL,
    "profileFirstText" text NOT NULL
);


--
-- Name: users; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.users (
    "UUID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    username character varying(32) NOT NULL,
    lastname character varying(32) NOT NULL,
    "passwordHash" character varying(32) NOT NULL,
    "passwordSalt" character varying(32) NOT NULL,
    "homeRegion" bigint,
    "homeLocationX" double precision,
    "homeLocationY" double precision,
    "homeLocationZ" double precision,
    "homeLookAtX" double precision,
    "homeLookAtY" double precision,
    "homeLookAtZ" double precision,
    created integer NOT NULL,
    "lastLogin" integer NOT NULL,
    "userInventoryURI" character varying(255) DEFAULT NULL::character varying,
    "userAssetURI" character varying(255) DEFAULT NULL::character varying,
    "profileCanDoMask" integer,
    "profileWantDoMask" integer,
    "profileAboutText" text,
    "profileFirstText" text,
    "profileImage" uuid,
    "profileFirstImage" uuid,
    "webLoginKey" uuid,
    "homeRegionID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    "userFlags" integer DEFAULT 0 NOT NULL,
    "godLevel" integer DEFAULT 0 NOT NULL,
    "customType" character varying(32) DEFAULT ''::character varying NOT NULL,
    partner uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL,
    email character varying(250),
    "scopeID" uuid DEFAULT '00000000-0000-0000-0000-000000000000'::uuid NOT NULL
);


--
-- Name: usersettings; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.usersettings (
    useruuid uuid NOT NULL,
    imviaemail boolean NOT NULL,
    visible boolean NOT NULL,
    email character varying(254) NOT NULL
);


--
-- Name: xassetsdata; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.xassetsdata (
    hash bytea NOT NULL,
    data bytea NOT NULL
);


--
-- Name: xassetsmeta; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.xassetsmeta (
    id uuid NOT NULL,
    hash bytea NOT NULL,
    name character varying(64) NOT NULL,
    description character varying(64) NOT NULL,
    "AssetType" smallint NOT NULL,
    local boolean NOT NULL,
    temporary boolean NOT NULL,
    create_time integer NOT NULL,
    access_time integer NOT NULL,
    asset_flags integer NOT NULL,
    creatorid character varying(128) NOT NULL
);


--
-- Name: im_offline ID; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.im_offline ALTER COLUMN "ID" SET DEFAULT nextval('public."im_offline_ID_seq"'::regclass);


--
-- Data for Name: agentprefs; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: agents; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: assets; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: auth; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: avatarappearance; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: avatarattachments; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: avatars; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: bakedterrain; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: classifieds; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: estate_allowed_experiences; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: estate_blocked_experiences; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: estate_groups; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: estate_key_experiences; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: estate_managers; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: estate_map; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: estate_settings; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: estate_users; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: estateban; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: friends; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: fsassets; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: griduser; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: hg_traveling_data; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: im_offline; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: inventoryfolders; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: inventoryitems; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: land; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: landaccesslist; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: logs; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: migrations; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.migrations VALUES ('migrations', 1);
INSERT INTO public.migrations VALUES ('AgentPrefs', 1);
INSERT INTO public.migrations VALUES ('AssetStore', 9);
INSERT INTO public.migrations VALUES ('AuthStore', 3);
INSERT INTO public.migrations VALUES ('Avatar', 3);
INSERT INTO public.migrations VALUES ('EstateStore', 16);
INSERT INTO public.migrations VALUES ('FSAssetStore', 1);
INSERT INTO public.migrations VALUES ('FriendsStore', 3);
INSERT INTO public.migrations VALUES ('GridStore', 10);
INSERT INTO public.migrations VALUES ('GridUserStore', 2);
INSERT INTO public.migrations VALUES ('HGTravelStore', 1);
INSERT INTO public.migrations VALUES ('IM_Store', 3);
INSERT INTO public.migrations VALUES ('InventoryStore', 11);
INSERT INTO public.migrations VALUES ('LogStore', 1);
INSERT INTO public.migrations VALUES ('MuteListStore', 2);
INSERT INTO public.migrations VALUES ('Presence', 4);
INSERT INTO public.migrations VALUES ('RegionStore', 62);
INSERT INTO public.migrations VALUES ('UserAccount', 7);
INSERT INTO public.migrations VALUES ('UserProfiles', 6);
INSERT INTO public.migrations VALUES ('UserStore', 11);
INSERT INTO public.migrations VALUES ('XAssetStore', 5);
INSERT INTO public.migrations VALUES ('os_groups_Store', 6);


--
-- Data for Name: mutelist; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: os_groups_bans; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: os_groups_groups; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: os_groups_invites; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: os_groups_membership; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: os_groups_notices; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: os_groups_principals; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: os_groups_rolemembership; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: os_groups_roles; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: presence; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: primitems; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: prims; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: primshapes; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: regionban; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: regionenvironment; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: regionextra; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: regions; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: regionsettings; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: regionwindlight; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: spawn_points; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: terrain; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: tokens; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: useraccounts; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: userdata; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: userfriends; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: usernotes; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: userpicks; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: userprofile; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: users; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: usersettings; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: xassetsdata; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: xassetsmeta; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Name: estate_settings_id; Type: SEQUENCE SET; Schema: public; Owner: -
--

SELECT pg_catalog.setval('public.estate_settings_id', 100, false);


--
-- Name: im_offiline_id; Type: SEQUENCE SET; Schema: public; Owner: -
--

SELECT pg_catalog.setval('public.im_offiline_id', 1, false);


--
-- Name: im_offline_ID_seq; Type: SEQUENCE SET; Schema: public; Owner: -
--

SELECT pg_catalog.setval('public."im_offline_ID_seq"', 1, false);


--
-- Name: agentprefs agentprefs_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.agentprefs
    ADD CONSTRAINT agentprefs_pkey PRIMARY KEY ("PrincipalID");


--
-- Name: assets assets_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.assets
    ADD CONSTRAINT assets_pkey PRIMARY KEY (id);


--
-- Name: classifieds classifiedspk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.classifieds
    ADD CONSTRAINT classifiedspk PRIMARY KEY (classifieduuid);


--
-- Name: estate_map estate_map_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.estate_map
    ADD CONSTRAINT estate_map_pkey PRIMARY KEY ("RegionID");


--
-- Name: estate_settings estate_settings_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.estate_settings
    ADD CONSTRAINT estate_settings_pkey PRIMARY KEY ("EstateID");


--
-- Name: friends friends_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.friends
    ADD CONSTRAINT friends_pkey PRIMARY KEY ("PrincipalID", "Friend");


--
-- Name: fsassets fsassets_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.fsassets
    ADD CONSTRAINT fsassets_pkey PRIMARY KEY (id);


--
-- Name: griduser griduser_tmp_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.griduser
    ADD CONSTRAINT griduser_tmp_pkey PRIMARY KEY ("UserID");


--
-- Name: hg_traveling_data hg_traveling_data_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.hg_traveling_data
    ADD CONSTRAINT hg_traveling_data_pkey PRIMARY KEY ("SessionID");


--
-- Name: im_offline im_offline_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.im_offline
    ADD CONSTRAINT im_offline_pkey PRIMARY KEY ("ID", "PrincipalID", "FromID");


--
-- Name: logs logs_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.logs
    ADD CONSTRAINT logs_pkey PRIMARY KEY ("logID");


--
-- Name: os_groups_invites os_groupinvitespk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.os_groups_invites
    ADD CONSTRAINT os_groupinvitespk PRIMARY KEY ("InviteID");


--
-- Name: os_groups_membership os_groupmemberpk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.os_groups_membership
    ADD CONSTRAINT os_groupmemberpk PRIMARY KEY ("GroupID", "PrincipalID");


--
-- Name: os_groups_principals os_groupprincpk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.os_groups_principals
    ADD CONSTRAINT os_groupprincpk PRIMARY KEY ("PrincipalID");


--
-- Name: os_groups_rolemembership os_grouprolememberpk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.os_groups_rolemembership
    ADD CONSTRAINT os_grouprolememberpk PRIMARY KEY ("GroupID", "RoleID", "PrincipalID");


--
-- Name: os_groups_roles os_grouprolepk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.os_groups_roles
    ADD CONSTRAINT os_grouprolepk PRIMARY KEY ("GroupID", "RoleID");


--
-- Name: os_groups_groups os_groups_groups_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.os_groups_groups
    ADD CONSTRAINT os_groups_groups_pkey PRIMARY KEY ("GroupID");


--
-- Name: os_groups_bans os_groupsbanspk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.os_groups_bans
    ADD CONSTRAINT os_groupsbanspk PRIMARY KEY ("GroupID", "PrincipalID");


--
-- Name: os_groups_notices os_groupsnoticespk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.os_groups_notices
    ADD CONSTRAINT os_groupsnoticespk PRIMARY KEY ("NoticeID");


--
-- Name: agents pk__agents__65a475e749c3f6b7; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.agents
    ADD CONSTRAINT pk__agents__65a475e749c3f6b7 PRIMARY KEY ("UUID");


--
-- Name: avatarappearance pk__avatarap__7dd115cc4e88abd4; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.avatarappearance
    ADD CONSTRAINT pk__avatarap__7dd115cc4e88abd4 PRIMARY KEY ("Owner");


--
-- Name: inventoryfolders pk__inventor__c2fabfb3173876ea; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inventoryfolders
    ADD CONSTRAINT pk__inventor__c2fabfb3173876ea PRIMARY KEY ("folderID");


--
-- Name: inventoryitems pk__inventor__c4b7bc2220c1e124; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inventoryitems
    ADD CONSTRAINT pk__inventor__c4b7bc2220c1e124 PRIMARY KEY ("inventoryID");


--
-- Name: land pk__land__65a475e71bfd2c07; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.land
    ADD CONSTRAINT pk__land__65a475e71bfd2c07 PRIMARY KEY ("UUID");


--
-- Name: primitems pk__primitems__0a688bb1; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.primitems
    ADD CONSTRAINT pk__primitems__0a688bb1 PRIMARY KEY ("itemID");


--
-- Name: prims pk__prims__10566f31; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.prims
    ADD CONSTRAINT pk__prims__10566f31 PRIMARY KEY ("UUID");


--
-- Name: primshapes pk__primshapes__0880433f; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.primshapes
    ADD CONSTRAINT pk__primshapes__0880433f PRIMARY KEY ("UUID");


--
-- Name: regions pk__regions__uuid; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.regions
    ADD CONSTRAINT pk__regions__uuid PRIMARY KEY (uuid);


--
-- Name: regionsettings pk__regionse__5b35159d21b6055d; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.regionsettings
    ADD CONSTRAINT pk__regionse__5b35159d21b6055d PRIMARY KEY ("regionUUID");


--
-- Name: users pk__users__65a475e737a5467c; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.users
    ADD CONSTRAINT pk__users__65a475e737a5467c PRIMARY KEY ("UUID");


--
-- Name: regionenvironment regionenvironment_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.regionenvironment
    ADD CONSTRAINT regionenvironment_pkey PRIMARY KEY (region_id);


--
-- Name: regionextra regionextra_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.regionextra
    ADD CONSTRAINT regionextra_pkey PRIMARY KEY ("RegionID", "Name");


--
-- Name: regionwindlight regionwindlight_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.regionwindlight
    ADD CONSTRAINT regionwindlight_pkey PRIMARY KEY (region_id);


--
-- Name: spawn_points spawn_points_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.spawn_points
    ADD CONSTRAINT spawn_points_pkey PRIMARY KEY ("RegionUUID", "Yaw", "Pitch", "Distance");


--
-- Name: avatars tmp_avatars_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.avatars
    ADD CONSTRAINT tmp_avatars_pkey PRIMARY KEY ("PrincipalID", "Name");


--
-- Name: mutelist unique_agent_2; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.mutelist
    ADD CONSTRAINT unique_agent_2 UNIQUE ("AgentID", "MuteID", "MuteName");


--
-- Name: useraccounts useraccounts_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.useraccounts
    ADD CONSTRAINT useraccounts_pkey PRIMARY KEY ("PrincipalID");


--
-- Name: userdata userdata_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.userdata
    ADD CONSTRAINT userdata_pkey PRIMARY KEY ("UserId", "TagId");


--
-- Name: usernotes usernoteuk; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.usernotes
    ADD CONSTRAINT usernoteuk UNIQUE (useruuid, targetuuid);


--
-- Name: userpicks userpicks_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.userpicks
    ADD CONSTRAINT userpicks_pkey PRIMARY KEY (pickuuid);


--
-- Name: userprofile userprofile_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.userprofile
    ADD CONSTRAINT userprofile_pkey PRIMARY KEY (useruuid);


--
-- Name: usersettings usersettings_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.usersettings
    ADD CONSTRAINT usersettings_pkey PRIMARY KEY (useruuid);


--
-- Name: tokens uuid_token; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tokens
    ADD CONSTRAINT uuid_token UNIQUE (uuid, token);


--
-- Name: xassetsdata xassetsdata_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.xassetsdata
    ADD CONSTRAINT xassetsdata_pkey PRIMARY KEY (hash);


--
-- Name: xassetsmeta xassetsmeta_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.xassetsmeta
    ADD CONSTRAINT xassetsmeta_pkey PRIMARY KEY (id);


--
-- Name: Email; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX "Email" ON public.useraccounts USING btree ("Email");


--
-- Name: FirstName; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX "FirstName" ON public.useraccounts USING btree ("FirstName");


--
-- Name: LastName; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX "LastName" ON public.useraccounts USING btree ("LastName");


--
-- Name: PrincipalID; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX "PrincipalID" ON public.useraccounts USING btree ("PrincipalID");


--
-- Name: auth_pkey; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX auth_pkey ON public.auth USING btree (uuid);


--
-- Name: flags; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX flags ON public.regions USING btree (flags);


--
-- Name: ii2_folder; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ii2_folder ON public.inventoryitems USING btree ("parentFolderID");


--
-- Name: ii2_owner; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ii2_owner ON public.inventoryitems USING btree ("avatarID");


--
-- Name: ix_avatarattachments; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_avatarattachments ON public.avatarattachments USING btree ("UUID");


--
-- Name: ix_estate_allowed_experiences; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_estate_allowed_experiences ON public.estate_allowed_experiences USING btree ("EstateID");


--
-- Name: ix_estate_blocked_experiences; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_estate_blocked_experiences ON public.estate_blocked_experiences USING btree ("EstateID");


--
-- Name: ix_estate_groups; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_estate_groups ON public.estate_groups USING btree ("EstateID");


--
-- Name: ix_estate_key_experiences; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_estate_key_experiences ON public.estate_key_experiences USING btree ("EstateID");


--
-- Name: ix_estate_managers; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_estate_managers ON public.estate_managers USING btree ("EstateID");


--
-- Name: ix_estate_users; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_estate_users ON public.estate_users USING btree ("EstateID");


--
-- Name: ix_estateban; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_estateban ON public.estateban USING btree ("EstateID");


--
-- Name: ix_mutelist_agentid; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_mutelist_agentid ON public.mutelist USING btree ("AgentID");


--
-- Name: ix_regions_handle; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_regions_handle ON public.regions USING btree ("regionHandle");


--
-- Name: ix_regions_name; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_regions_name ON public.regions USING btree ("regionName");


--
-- Name: ix_regions_override; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_regions_override ON public.regions USING btree ("eastOverrideHandle", "westOverrideHandle", "southOverrideHandle", "northOverrideHandle");


--
-- Name: ix_userfriends_friendid; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_userfriends_friendid ON public.userfriends USING btree ("friendID");


--
-- Name: ix_userfriends_ownerid; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_userfriends_ownerid ON public.userfriends USING btree ("ownerID");


--
-- Name: land_lower_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX land_lower_idx ON public.land USING btree (lower(("Name")::text));


--
-- Name: land_regionuuid; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX land_regionuuid ON public.land USING btree ("RegionUUID");


--
-- Name: landaccesslist_landuuid; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX landaccesslist_landuuid ON public.landaccesslist USING btree ("LandUUID");


--
-- Name: name; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX name ON public.useraccounts USING btree ("FirstName", "LastName");


--
-- Name: os_groups_membership_principal; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX os_groups_membership_principal ON public.os_groups_membership USING btree ("PrincipalID");


--
-- Name: os_groups_notices_group; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX os_groups_notices_group ON public.os_groups_notices USING btree ("GroupID");


--
-- Name: os_groups_notices_stamp; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX os_groups_notices_stamp ON public.os_groups_notices USING btree ("TMStamp");


--
-- Name: os_groups_rolemembership_member; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX os_groups_rolemembership_member ON public.os_groups_rolemembership USING btree ("GroupID", "PrincipalID");


--
-- Name: os_groups_rolemembership_principal; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX os_groups_rolemembership_principal ON public.os_groups_rolemembership USING btree ("PrincipalID");


--
-- Name: owner; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX owner ON public.inventoryfolders USING btree ("agentID");


--
-- Name: parent; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX parent ON public.inventoryfolders USING btree ("parentFolderID");


--
-- Name: primitems_primid; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX primitems_primid ON public.primitems USING btree ("primID");


--
-- Name: prims_parentid; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX prims_parentid ON public.prims USING btree ("ParentID");


--
-- Name: prims_regionuuid; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX prims_regionuuid ON public.prims USING btree ("RegionUUID");


--
-- Name: regionid; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX regionid ON public.presence USING btree ("RegionID");


--
-- Name: session; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX session ON public.agents USING btree ("sessionID");


--
-- Name: sessionid; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX sessionid ON public.presence USING btree ("SessionID");


--
-- Name: ssession; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ssession ON public.agents USING btree ("secureSessionID");


--
-- Name: userid; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX userid ON public.presence USING btree ("UserID");


--
-- Name: usernames; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX usernames ON public.users USING btree (username, lastname);


--
-- PostgreSQL database dump complete
--


