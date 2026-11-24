
/*!40101 SET @OLD_CHARACTER_SET_CLIENT=@@CHARACTER_SET_CLIENT */;
/*!40101 SET @OLD_CHARACTER_SET_RESULTS=@@CHARACTER_SET_RESULTS */;
/*!40101 SET @OLD_COLLATION_CONNECTION=@@COLLATION_CONNECTION */;
/*!50503 SET NAMES utf8mb4 */;
/*!40103 SET @OLD_TIME_ZONE=@@TIME_ZONE */;
/*!40103 SET TIME_ZONE='+00:00' */;
/*!40014 SET @OLD_UNIQUE_CHECKS=@@UNIQUE_CHECKS, UNIQUE_CHECKS=0 */;
/*!40014 SET @OLD_FOREIGN_KEY_CHECKS=@@FOREIGN_KEY_CHECKS, FOREIGN_KEY_CHECKS=0 */;
/*!40101 SET @OLD_SQL_MODE=@@SQL_MODE, SQL_MODE='NO_AUTO_VALUE_ON_ZERO' */;
/*!40111 SET @OLD_SQL_NOTES=@@SQL_NOTES, SQL_NOTES=0 */;
DROP TABLE IF EXISTS `AgentPrefs`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `AgentPrefs` (
  `PrincipalID` char(36) NOT NULL,
  `AccessPrefs` char(2) NOT NULL DEFAULT 'M',
  `HoverHeight` double(30,27) NOT NULL DEFAULT '0.000000000000000000000000000',
  `Language` char(5) NOT NULL DEFAULT 'en-us',
  `LanguageIsPublic` tinyint(1) NOT NULL DEFAULT '1',
  `PermEveryone` int NOT NULL DEFAULT '0',
  `PermGroup` int NOT NULL DEFAULT '0',
  `PermNextOwner` int NOT NULL DEFAULT '532480',
  PRIMARY KEY (`PrincipalID`),
  UNIQUE KEY `PrincipalID` (`PrincipalID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `AgentPrefs` DISABLE KEYS */;
/*!40000 ALTER TABLE `AgentPrefs` ENABLE KEYS */;
DROP TABLE IF EXISTS `Avatars`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `Avatars` (
  `PrincipalID` char(36) NOT NULL,
  `Name` varchar(32) NOT NULL,
  `Value` text,
  PRIMARY KEY (`PrincipalID`,`Name`),
  KEY `PrincipalID` (`PrincipalID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `Avatars` DISABLE KEYS */;
/*!40000 ALTER TABLE `Avatars` ENABLE KEYS */;
DROP TABLE IF EXISTS `Friends`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `Friends` (
  `PrincipalID` varchar(255) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `Friend` varchar(255) NOT NULL,
  `Flags` varchar(16) NOT NULL DEFAULT '0',
  `Offered` varchar(32) NOT NULL DEFAULT '0',
  PRIMARY KEY (`PrincipalID`(36),`Friend`(36)),
  KEY `PrincipalID` (`PrincipalID`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `Friends` DISABLE KEYS */;
/*!40000 ALTER TABLE `Friends` ENABLE KEYS */;
DROP TABLE IF EXISTS `GridUser`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `GridUser` (
  `UserID` varchar(255) NOT NULL,
  `HomeRegionID` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `HomePosition` char(64) NOT NULL DEFAULT '<0,0,0>',
  `HomeLookAt` char(64) NOT NULL DEFAULT '<0,0,0>',
  `LastRegionID` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `LastPosition` char(64) NOT NULL DEFAULT '<0,0,0>',
  `LastLookAt` char(64) NOT NULL DEFAULT '<0,0,0>',
  `Online` char(5) NOT NULL DEFAULT 'false',
  `Login` char(16) NOT NULL DEFAULT '0',
  `Logout` char(16) NOT NULL DEFAULT '0',
  PRIMARY KEY (`UserID`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `GridUser` DISABLE KEYS */;
/*!40000 ALTER TABLE `GridUser` ENABLE KEYS */;
DROP TABLE IF EXISTS `MuteList`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `MuteList` (
  `AgentID` char(36) NOT NULL,
  `MuteID` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `MuteName` varchar(64) NOT NULL DEFAULT '',
  `MuteType` int NOT NULL DEFAULT '1',
  `MuteFlags` int NOT NULL DEFAULT '0',
  `Stamp` int NOT NULL,
  UNIQUE KEY `AgentID_2` (`AgentID`,`MuteID`,`MuteName`),
  KEY `AgentID` (`AgentID`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `MuteList` DISABLE KEYS */;
/*!40000 ALTER TABLE `MuteList` ENABLE KEYS */;
DROP TABLE IF EXISTS `Presence`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `Presence` (
  `UserID` varchar(255) NOT NULL,
  `RegionID` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `SessionID` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `SecureSessionID` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `LastSeen` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
  UNIQUE KEY `SessionID` (`SessionID`),
  KEY `UserID` (`UserID`),
  KEY `RegionID` (`RegionID`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `Presence` DISABLE KEYS */;
/*!40000 ALTER TABLE `Presence` ENABLE KEYS */;
DROP TABLE IF EXISTS `UserAccounts`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `UserAccounts` (
  `PrincipalID` char(36) NOT NULL,
  `ScopeID` char(36) NOT NULL,
  `FirstName` varchar(64) NOT NULL,
  `LastName` varchar(64) NOT NULL,
  `Email` varchar(64) DEFAULT NULL,
  `ServiceURLs` text,
  `Created` int DEFAULT NULL,
  `UserLevel` int NOT NULL DEFAULT '0',
  `UserFlags` int NOT NULL DEFAULT '0',
  `UserTitle` varchar(64) CHARACTER SET utf8mb3 COLLATE utf8mb3_general_ci NOT NULL DEFAULT '',
  `active` int NOT NULL DEFAULT '1',
  `DisplayName` varchar(31) DEFAULT NULL,
  `NameChanged` int DEFAULT '0',
  `UserCountry` varchar(100) NOT NULL DEFAULT '',
  UNIQUE KEY `PrincipalID` (`PrincipalID`),
  KEY `Email` (`Email`),
  KEY `FirstName` (`FirstName`),
  KEY `LastName` (`LastName`),
  KEY `Name` (`FirstName`,`LastName`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `UserAccounts` DISABLE KEYS */;
/*!40000 ALTER TABLE `UserAccounts` ENABLE KEYS */;
DROP TABLE IF EXISTS `UserAlias`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `UserAlias` (
  `Id` int NOT NULL AUTO_INCREMENT,
  `AliasID` char(36) NOT NULL,
  `UserID` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `Description` varchar(80) DEFAULT NULL,
  UNIQUE KEY `Id` (`Id`),
  UNIQUE KEY `AliasID` (`AliasID`),
  KEY `UserID` (`UserID`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `UserAlias` DISABLE KEYS */;
/*!40000 ALTER TABLE `UserAlias` ENABLE KEYS */;
DROP TABLE IF EXISTS `XAssetsData`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `XAssetsData` (
  `Hash` binary(32) NOT NULL,
  `Data` longblob NOT NULL,
  PRIMARY KEY (`Hash`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3 COMMENT='Version 1';
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `XAssetsData` DISABLE KEYS */;
/*!40000 ALTER TABLE `XAssetsData` ENABLE KEYS */;
DROP TABLE IF EXISTS `XAssetsMeta`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `XAssetsMeta` (
  `ID` char(36) NOT NULL,
  `Hash` binary(32) NOT NULL,
  `Name` varchar(64) NOT NULL,
  `Description` varchar(64) NOT NULL,
  `AssetType` tinyint NOT NULL,
  `Local` tinyint(1) NOT NULL,
  `Temporary` tinyint(1) NOT NULL,
  `CreateTime` int NOT NULL,
  `AccessTime` int NOT NULL,
  `AssetFlags` int NOT NULL,
  `CreatorID` varchar(128) NOT NULL,
  PRIMARY KEY (`ID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3 COMMENT='Version 1';
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `XAssetsMeta` DISABLE KEYS */;
/*!40000 ALTER TABLE `XAssetsMeta` ENABLE KEYS */;
DROP TABLE IF EXISTS `assets`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `assets` (
  `name` varchar(64) NOT NULL,
  `description` varchar(64) NOT NULL,
  `assetType` tinyint NOT NULL,
  `local` tinyint(1) NOT NULL,
  `temporary` tinyint(1) NOT NULL,
  `data` longblob NOT NULL,
  `id` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `create_time` int DEFAULT '0',
  `access_time` int DEFAULT '0',
  `asset_flags` int NOT NULL DEFAULT '0',
  `CreatorID` varchar(128) NOT NULL DEFAULT '',
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `assets` DISABLE KEYS */;
/*!40000 ALTER TABLE `assets` ENABLE KEYS */;
DROP TABLE IF EXISTS `auth`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `auth` (
  `UUID` char(36) NOT NULL,
  `passwordHash` char(32) NOT NULL DEFAULT '',
  `passwordSalt` char(32) NOT NULL DEFAULT '',
  `webLoginKey` varchar(255) NOT NULL DEFAULT '',
  `accountType` varchar(32) NOT NULL DEFAULT 'UserAccount',
  PRIMARY KEY (`UUID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `auth` DISABLE KEYS */;
/*!40000 ALTER TABLE `auth` ENABLE KEYS */;
DROP TABLE IF EXISTS `bakedterrain`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `bakedterrain` (
  `RegionUUID` varchar(255) DEFAULT NULL,
  `Revision` int DEFAULT NULL,
  `Heightfield` longblob
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `bakedterrain` DISABLE KEYS */;
/*!40000 ALTER TABLE `bakedterrain` ENABLE KEYS */;
DROP TABLE IF EXISTS `classifieds`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `classifieds` (
  `classifieduuid` char(36) NOT NULL,
  `creatoruuid` char(36) NOT NULL,
  `creationdate` int NOT NULL,
  `expirationdate` int NOT NULL,
  `category` varchar(20) NOT NULL,
  `name` varchar(255) NOT NULL,
  `description` text NOT NULL,
  `parceluuid` char(36) NOT NULL,
  `parentestate` int NOT NULL,
  `snapshotuuid` char(36) NOT NULL,
  `simname` varchar(255) NOT NULL,
  `posglobal` varchar(255) NOT NULL,
  `parcelname` varchar(255) NOT NULL,
  `classifiedflags` int NOT NULL,
  `priceforlisting` int NOT NULL,
  PRIMARY KEY (`classifieduuid`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `classifieds` DISABLE KEYS */;
/*!40000 ALTER TABLE `classifieds` ENABLE KEYS */;
DROP TABLE IF EXISTS `estate_allowed_experiences`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `estate_allowed_experiences` (
  `EstateID` int unsigned NOT NULL,
  `uuid` char(36) NOT NULL,
  KEY `EstateID` (`EstateID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `estate_allowed_experiences` DISABLE KEYS */;
/*!40000 ALTER TABLE `estate_allowed_experiences` ENABLE KEYS */;
DROP TABLE IF EXISTS `estate_blocked_experiences`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `estate_blocked_experiences` (
  `EstateID` int unsigned NOT NULL,
  `uuid` char(36) NOT NULL,
  KEY `EstateID` (`EstateID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `estate_blocked_experiences` DISABLE KEYS */;
/*!40000 ALTER TABLE `estate_blocked_experiences` ENABLE KEYS */;
DROP TABLE IF EXISTS `estate_groups`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `estate_groups` (
  `EstateID` int unsigned NOT NULL,
  `uuid` char(36) NOT NULL,
  KEY `EstateID` (`EstateID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `estate_groups` DISABLE KEYS */;
/*!40000 ALTER TABLE `estate_groups` ENABLE KEYS */;
DROP TABLE IF EXISTS `estate_key_experiences`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `estate_key_experiences` (
  `EstateID` int unsigned NOT NULL,
  `uuid` char(36) NOT NULL,
  KEY `EstateID` (`EstateID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `estate_key_experiences` DISABLE KEYS */;
/*!40000 ALTER TABLE `estate_key_experiences` ENABLE KEYS */;
DROP TABLE IF EXISTS `estate_managers`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `estate_managers` (
  `EstateID` int unsigned NOT NULL,
  `uuid` char(36) NOT NULL,
  KEY `EstateID` (`EstateID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `estate_managers` DISABLE KEYS */;
/*!40000 ALTER TABLE `estate_managers` ENABLE KEYS */;
DROP TABLE IF EXISTS `estate_map`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `estate_map` (
  `RegionID` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `EstateID` int NOT NULL,
  PRIMARY KEY (`RegionID`),
  KEY `EstateID` (`EstateID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `estate_map` DISABLE KEYS */;
/*!40000 ALTER TABLE `estate_map` ENABLE KEYS */;
DROP TABLE IF EXISTS `estate_settings`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `estate_settings` (
  `EstateID` int unsigned NOT NULL AUTO_INCREMENT,
  `EstateName` varchar(64) DEFAULT NULL,
  `AbuseEmailToEstateOwner` tinyint NOT NULL,
  `DenyAnonymous` tinyint NOT NULL,
  `ResetHomeOnTeleport` tinyint NOT NULL,
  `FixedSun` tinyint NOT NULL,
  `DenyTransacted` tinyint NOT NULL,
  `BlockDwell` tinyint NOT NULL,
  `DenyIdentified` tinyint NOT NULL,
  `AllowVoice` tinyint NOT NULL,
  `UseGlobalTime` tinyint NOT NULL,
  `PricePerMeter` int NOT NULL,
  `TaxFree` tinyint NOT NULL,
  `AllowDirectTeleport` tinyint NOT NULL,
  `RedirectGridX` int NOT NULL,
  `RedirectGridY` int NOT NULL,
  `ParentEstateID` int unsigned NOT NULL,
  `SunPosition` double NOT NULL,
  `EstateSkipScripts` tinyint NOT NULL,
  `BillableFactor` float NOT NULL,
  `PublicAccess` tinyint NOT NULL,
  `AbuseEmail` varchar(255) NOT NULL,
  `EstateOwner` varchar(36) NOT NULL,
  `DenyMinors` tinyint NOT NULL,
  `AllowLandmark` tinyint NOT NULL DEFAULT '1',
  `AllowParcelChanges` tinyint NOT NULL DEFAULT '1',
  `AllowSetHome` tinyint NOT NULL DEFAULT '1',
  `AllowEnviromentOverride` tinyint NOT NULL DEFAULT '0',
  PRIMARY KEY (`EstateID`)
) ENGINE=InnoDB AUTO_INCREMENT=101 DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `estate_settings` DISABLE KEYS */;
/*!40000 ALTER TABLE `estate_settings` ENABLE KEYS */;
DROP TABLE IF EXISTS `estate_users`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `estate_users` (
  `EstateID` int unsigned NOT NULL,
  `uuid` char(36) NOT NULL,
  KEY `EstateID` (`EstateID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `estate_users` DISABLE KEYS */;
/*!40000 ALTER TABLE `estate_users` ENABLE KEYS */;
DROP TABLE IF EXISTS `estateban`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `estateban` (
  `EstateID` int unsigned NOT NULL,
  `bannedUUID` varchar(36) NOT NULL,
  `bannedIp` varchar(16) NOT NULL,
  `bannedIpHostMask` varchar(16) NOT NULL,
  `bannedNameMask` varchar(64) DEFAULT NULL,
  `banningUUID` varchar(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `banTime` int NOT NULL DEFAULT '0',
  KEY `estateban_EstateID` (`EstateID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `estateban` DISABLE KEYS */;
/*!40000 ALTER TABLE `estateban` ENABLE KEYS */;
DROP TABLE IF EXISTS `experience_kv`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `experience_kv` (
  `experience` varchar(36) NOT NULL,
  `key` varchar(1011) CHARACTER SET latin1 COLLATE latin1_bin NOT NULL,
  `value` varchar(4095) CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci NOT NULL,
  PRIMARY KEY (`experience`,`key`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `experience_kv` DISABLE KEYS */;
/*!40000 ALTER TABLE `experience_kv` ENABLE KEYS */;
DROP TABLE IF EXISTS `experience_permissions`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `experience_permissions` (
  `experience` varchar(36) CHARACTER SET utf8mb3 COLLATE utf8mb3_unicode_ci NOT NULL,
  `avatar` varchar(36) CHARACTER SET utf8mb3 COLLATE utf8mb3_unicode_ci NOT NULL,
  `allow` bit(1) NOT NULL,
  PRIMARY KEY (`experience`,`avatar`) USING BTREE,
  KEY `avatar` (`avatar`) USING BTREE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3 COLLATE=utf8mb3_unicode_ci;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `experience_permissions` DISABLE KEYS */;
/*!40000 ALTER TABLE `experience_permissions` ENABLE KEYS */;
DROP TABLE IF EXISTS `experiences`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `experiences` (
  `public_id` varchar(36) COLLATE utf8mb3_unicode_ci NOT NULL,
  `owner_id` varchar(36) COLLATE utf8mb3_unicode_ci NOT NULL,
  `name` varchar(42) COLLATE utf8mb3_unicode_ci NOT NULL,
  `description` varchar(128) COLLATE utf8mb3_unicode_ci NOT NULL,
  `group_id` varchar(36) COLLATE utf8mb3_unicode_ci NOT NULL,
  `logo` varchar(36) COLLATE utf8mb3_unicode_ci NOT NULL,
  `marketplace` varchar(256) COLLATE utf8mb3_unicode_ci NOT NULL,
  `slurl` varchar(256) COLLATE utf8mb3_unicode_ci NOT NULL,
  `maturity` int NOT NULL,
  `properties` int NOT NULL,
  PRIMARY KEY (`public_id`) USING BTREE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3 COLLATE=utf8mb3_unicode_ci;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `experiences` DISABLE KEYS */;
/*!40000 ALTER TABLE `experiences` ENABLE KEYS */;
DROP TABLE IF EXISTS `fsassets`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `fsassets` (
  `id` char(36) NOT NULL,
  `name` varchar(64) NOT NULL DEFAULT '',
  `description` varchar(64) NOT NULL DEFAULT '',
  `type` int NOT NULL,
  `hash` char(80) NOT NULL,
  `create_time` int NOT NULL DEFAULT '0',
  `access_time` int NOT NULL DEFAULT '0',
  `asset_flags` int NOT NULL DEFAULT '0',
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `fsassets` DISABLE KEYS */;
/*!40000 ALTER TABLE `fsassets` ENABLE KEYS */;
DROP TABLE IF EXISTS `hg_grid_aliases`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `hg_grid_aliases` (
  `grid_id` char(36) NOT NULL,
  `alias_uri` varchar(255) NOT NULL,
  PRIMARY KEY (`alias_uri`),
  KEY `grid_id` (`grid_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `hg_grid_aliases` DISABLE KEYS */;
/*!40000 ALTER TABLE `hg_grid_aliases` ENABLE KEYS */;
DROP TABLE IF EXISTS `hg_traveling_data`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `hg_traveling_data` (
  `SessionID` varchar(36) NOT NULL,
  `UserID` varchar(36) NOT NULL,
  `GridExternalName` varchar(255) NOT NULL DEFAULT '',
  `ServiceToken` varchar(255) NOT NULL DEFAULT '',
  `ClientIPAddress` varchar(16) NOT NULL DEFAULT '',
  `MyIPAddress` varchar(16) NOT NULL DEFAULT '',
  `TMStamp` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (`SessionID`),
  KEY `UserID` (`UserID`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `hg_traveling_data` DISABLE KEYS */;
/*!40000 ALTER TABLE `hg_traveling_data` ENABLE KEYS */;
DROP TABLE IF EXISTS `hg_trusted_grids`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `hg_trusted_grids` (
  `id` char(36) NOT NULL,
  `home_uri` varchar(255) NOT NULL,
  `public_key` varbinary(32) DEFAULT NULL,
  `key_fingerprint` char(64) NOT NULL DEFAULT '',
  `tier` tinyint NOT NULL DEFAULT '1',
  `state` tinyint NOT NULL DEFAULT '0',
  `first_seen` datetime NOT NULL,
  `last_seen` datetime NOT NULL,
  `approved_by` varchar(64) NOT NULL DEFAULT '',
  `approved_at` datetime DEFAULT NULL,
  `notes` text,
  PRIMARY KEY (`id`),
  UNIQUE KEY `home_uri` (`home_uri`),
  KEY `key_fingerprint` (`key_fingerprint`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `hg_trusted_grids` DISABLE KEYS */;
/*!40000 ALTER TABLE `hg_trusted_grids` ENABLE KEYS */;
DROP TABLE IF EXISTS `im_offline`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `im_offline` (
  `ID` mediumint NOT NULL AUTO_INCREMENT,
  `PrincipalID` char(36) NOT NULL DEFAULT '',
  `FromID` char(36) NOT NULL DEFAULT '',
  `Message` text NOT NULL,
  `TMStamp` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
  PRIMARY KEY (`ID`),
  KEY `PrincipalID` (`PrincipalID`),
  KEY `FromID` (`FromID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `im_offline` DISABLE KEYS */;
/*!40000 ALTER TABLE `im_offline` ENABLE KEYS */;
DROP TABLE IF EXISTS `inventoryfolders`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `inventoryfolders` (
  `folderName` varchar(64) DEFAULT NULL,
  `type` smallint NOT NULL DEFAULT '0',
  `version` int NOT NULL DEFAULT '0',
  `folderID` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `agentID` char(36) DEFAULT NULL,
  `parentFolderID` char(36) DEFAULT NULL,
  PRIMARY KEY (`folderID`),
  KEY `inventoryfolders_agentid` (`agentID`),
  KEY `inventoryfolders_parentFolderid` (`parentFolderID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `inventoryfolders` DISABLE KEYS */;
/*!40000 ALTER TABLE `inventoryfolders` ENABLE KEYS */;
DROP TABLE IF EXISTS `inventoryitems`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `inventoryitems` (
  `assetID` varchar(36) DEFAULT NULL,
  `assetType` int DEFAULT NULL,
  `inventoryName` varchar(64) DEFAULT NULL,
  `inventoryDescription` varchar(128) DEFAULT NULL,
  `inventoryNextPermissions` int unsigned DEFAULT NULL,
  `inventoryCurrentPermissions` int unsigned DEFAULT NULL,
  `invType` int DEFAULT NULL,
  `creatorID` varchar(255) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `inventoryBasePermissions` int unsigned NOT NULL DEFAULT '0',
  `inventoryEveryOnePermissions` int unsigned NOT NULL DEFAULT '0',
  `salePrice` int NOT NULL DEFAULT '0',
  `saleType` tinyint NOT NULL DEFAULT '0',
  `creationDate` int NOT NULL DEFAULT '0',
  `groupID` varchar(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `groupOwned` tinyint NOT NULL DEFAULT '0',
  `flags` int unsigned NOT NULL DEFAULT '0',
  `inventoryID` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `avatarID` char(36) DEFAULT NULL,
  `parentFolderID` char(36) DEFAULT NULL,
  `inventoryGroupPermissions` int unsigned NOT NULL DEFAULT '0',
  `experienceID` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  PRIMARY KEY (`inventoryID`),
  KEY `inventoryitems_avatarid` (`avatarID`),
  KEY `inventoryitems_parentFolderid` (`parentFolderID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `inventoryitems` DISABLE KEYS */;
/*!40000 ALTER TABLE `inventoryitems` ENABLE KEYS */;
DROP TABLE IF EXISTS `land`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `land` (
  `UUID` varchar(255) NOT NULL,
  `RegionUUID` varchar(255) DEFAULT NULL,
  `LocalLandID` int DEFAULT NULL,
  `Bitmap` longblob,
  `Name` varchar(255) DEFAULT NULL,
  `Description` varchar(255) DEFAULT NULL,
  `OwnerUUID` varchar(255) DEFAULT NULL,
  `IsGroupOwned` int DEFAULT NULL,
  `Area` int DEFAULT NULL,
  `AuctionID` int DEFAULT NULL,
  `Category` int DEFAULT NULL,
  `ClaimDate` int DEFAULT NULL,
  `ClaimPrice` int DEFAULT NULL,
  `GroupUUID` varchar(255) DEFAULT NULL,
  `SalePrice` int DEFAULT NULL,
  `LandStatus` int DEFAULT NULL,
  `LandFlags` int unsigned DEFAULT NULL,
  `LandingType` int DEFAULT NULL,
  `MediaAutoScale` int DEFAULT NULL,
  `MediaTextureUUID` varchar(255) DEFAULT NULL,
  `MediaURL` varchar(255) DEFAULT NULL,
  `MusicURL` varchar(255) DEFAULT NULL,
  `PassHours` float DEFAULT NULL,
  `PassPrice` int DEFAULT NULL,
  `SnapshotUUID` varchar(255) DEFAULT NULL,
  `UserLocationX` float DEFAULT NULL,
  `UserLocationY` float DEFAULT NULL,
  `UserLocationZ` float DEFAULT NULL,
  `UserLookAtX` float DEFAULT NULL,
  `UserLookAtY` float DEFAULT NULL,
  `UserLookAtZ` float DEFAULT NULL,
  `AuthbuyerID` varchar(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `OtherCleanTime` int NOT NULL DEFAULT '0',
  `Dwell` int NOT NULL DEFAULT '0',
  `MediaType` varchar(32) NOT NULL DEFAULT 'none/none',
  `MediaDescription` varchar(255) NOT NULL DEFAULT '',
  `MediaSize` varchar(16) NOT NULL DEFAULT '0,0',
  `MediaLoop` tinyint(1) NOT NULL DEFAULT '0',
  `ObscureMusic` tinyint(1) NOT NULL DEFAULT '0',
  `ObscureMedia` tinyint(1) NOT NULL DEFAULT '0',
  `SeeAVs` tinyint NOT NULL DEFAULT '1',
  `AnyAVSounds` tinyint NOT NULL DEFAULT '1',
  `GroupAVSounds` tinyint NOT NULL DEFAULT '1',
  `environment` mediumtext,
  PRIMARY KEY (`UUID`),
  KEY `land_regionuuid` (`RegionUUID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `land` DISABLE KEYS */;
/*!40000 ALTER TABLE `land` ENABLE KEYS */;
DROP TABLE IF EXISTS `landaccesslist`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `landaccesslist` (
  `LandUUID` varchar(255) DEFAULT NULL,
  `AccessUUID` varchar(255) DEFAULT NULL,
  `Flags` int DEFAULT NULL,
  `Expires` int NOT NULL DEFAULT '0',
  KEY `landaccesslist_landuuid` (`LandUUID`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `landaccesslist` DISABLE KEYS */;
/*!40000 ALTER TABLE `landaccesslist` ENABLE KEYS */;
DROP TABLE IF EXISTS `logs`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `logs` (
  `logID` int unsigned NOT NULL AUTO_INCREMENT,
  `target` varchar(36) DEFAULT NULL,
  `server` varchar(64) DEFAULT NULL,
  `method` varchar(64) DEFAULT NULL,
  `arguments` varchar(255) DEFAULT NULL,
  `priority` int DEFAULT NULL,
  `message` text,
  PRIMARY KEY (`logID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `logs` DISABLE KEYS */;
/*!40000 ALTER TABLE `logs` ENABLE KEYS */;
DROP TABLE IF EXISTS `migrations`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `migrations` (
  `name` varchar(100) DEFAULT NULL,
  `version` int DEFAULT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `migrations` DISABLE KEYS */;
INSERT INTO `migrations` VALUES ('migrations',1),('AgentPrefs',1),('AssetStore',10),('AuthStore',4),('Avatar',3),('EstateStore',38),('Experience',4),('FSAssetStore',1),('FriendsStore',4),('GridStore',11),('GridUserStore',2),('HGTravelStore',2),('IM_Store',5),('InventoryStore',8),('LogStore',1),('MuteListStore',1),('Presence',4),('RegionStore',71),('TrustedGrid',1),('UserAccount',8),('UserAlias',1),('UserProfiles',5),('XAssetStore',2),('os_groups_Store',5);
/*!40000 ALTER TABLE `migrations` ENABLE KEYS */;
DROP TABLE IF EXISTS `os_groups_bans`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `os_groups_bans` (
  `GroupID` char(36) NOT NULL DEFAULT '',
  `PrincipalID` varchar(255) NOT NULL DEFAULT '',
  `BanTime` int NOT NULL DEFAULT '0',
  PRIMARY KEY (`GroupID`,`PrincipalID`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `os_groups_bans` DISABLE KEYS */;
/*!40000 ALTER TABLE `os_groups_bans` ENABLE KEYS */;
DROP TABLE IF EXISTS `os_groups_groups`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `os_groups_groups` (
  `GroupID` char(36) NOT NULL DEFAULT '',
  `Location` varchar(255) NOT NULL DEFAULT '',
  `Name` varchar(255) CHARACTER SET utf8mb3 COLLATE utf8mb3_general_ci NOT NULL DEFAULT '',
  `Charter` text CHARACTER SET utf8mb3 COLLATE utf8mb3_general_ci NOT NULL,
  `InsigniaID` char(36) NOT NULL DEFAULT '',
  `FounderID` char(36) NOT NULL DEFAULT '',
  `MembershipFee` int NOT NULL DEFAULT '0',
  `OpenEnrollment` varchar(255) NOT NULL DEFAULT '',
  `ShowInList` int NOT NULL DEFAULT '0',
  `AllowPublish` int NOT NULL DEFAULT '0',
  `MaturePublish` int NOT NULL DEFAULT '0',
  `OwnerRoleID` char(36) NOT NULL DEFAULT '',
  PRIMARY KEY (`GroupID`),
  UNIQUE KEY `Name` (`Name`),
  FULLTEXT KEY `Name_2` (`Name`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `os_groups_groups` DISABLE KEYS */;
/*!40000 ALTER TABLE `os_groups_groups` ENABLE KEYS */;
DROP TABLE IF EXISTS `os_groups_invites`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `os_groups_invites` (
  `InviteID` char(36) NOT NULL DEFAULT '',
  `GroupID` char(36) NOT NULL DEFAULT '',
  `RoleID` char(36) NOT NULL DEFAULT '',
  `PrincipalID` varchar(255) NOT NULL DEFAULT '',
  `TMStamp` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (`InviteID`),
  UNIQUE KEY `PrincipalGroup` (`GroupID`,`PrincipalID`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `os_groups_invites` DISABLE KEYS */;
/*!40000 ALTER TABLE `os_groups_invites` ENABLE KEYS */;
DROP TABLE IF EXISTS `os_groups_membership`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `os_groups_membership` (
  `GroupID` char(36) NOT NULL DEFAULT '',
  `PrincipalID` varchar(255) NOT NULL DEFAULT '',
  `SelectedRoleID` char(36) NOT NULL DEFAULT '',
  `Contribution` int NOT NULL DEFAULT '0',
  `ListInProfile` int NOT NULL DEFAULT '1',
  `AcceptNotices` int NOT NULL DEFAULT '1',
  `AccessToken` char(36) NOT NULL DEFAULT '',
  PRIMARY KEY (`GroupID`,`PrincipalID`),
  KEY `PrincipalID` (`PrincipalID`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `os_groups_membership` DISABLE KEYS */;
/*!40000 ALTER TABLE `os_groups_membership` ENABLE KEYS */;
DROP TABLE IF EXISTS `os_groups_notices`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `os_groups_notices` (
  `GroupID` char(36) NOT NULL DEFAULT '',
  `NoticeID` char(36) NOT NULL DEFAULT '',
  `TMStamp` int unsigned NOT NULL DEFAULT '0',
  `FromName` varchar(255) NOT NULL DEFAULT '',
  `Subject` varchar(255) CHARACTER SET utf8mb3 COLLATE utf8mb3_general_ci NOT NULL DEFAULT '',
  `Message` text CHARACTER SET utf8mb3 COLLATE utf8mb3_general_ci NOT NULL,
  `HasAttachment` int NOT NULL DEFAULT '0',
  `AttachmentType` int NOT NULL DEFAULT '0',
  `AttachmentName` varchar(128) NOT NULL DEFAULT '',
  `AttachmentItemID` char(36) NOT NULL DEFAULT '',
  `AttachmentOwnerID` varchar(255) NOT NULL DEFAULT '',
  PRIMARY KEY (`NoticeID`),
  KEY `GroupID` (`GroupID`),
  KEY `TMStamp` (`TMStamp`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `os_groups_notices` DISABLE KEYS */;
/*!40000 ALTER TABLE `os_groups_notices` ENABLE KEYS */;
DROP TABLE IF EXISTS `os_groups_principals`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `os_groups_principals` (
  `PrincipalID` varchar(255) NOT NULL DEFAULT '',
  `ActiveGroupID` char(36) NOT NULL DEFAULT '',
  PRIMARY KEY (`PrincipalID`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `os_groups_principals` DISABLE KEYS */;
/*!40000 ALTER TABLE `os_groups_principals` ENABLE KEYS */;
DROP TABLE IF EXISTS `os_groups_rolemembership`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `os_groups_rolemembership` (
  `GroupID` char(36) NOT NULL DEFAULT '',
  `RoleID` char(36) NOT NULL DEFAULT '',
  `PrincipalID` varchar(255) NOT NULL DEFAULT '',
  PRIMARY KEY (`GroupID`,`RoleID`,`PrincipalID`),
  KEY `PrincipalID` (`PrincipalID`),
  KEY `GroupPrincipal` (`GroupID`,`PrincipalID`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `os_groups_rolemembership` DISABLE KEYS */;
/*!40000 ALTER TABLE `os_groups_rolemembership` ENABLE KEYS */;
DROP TABLE IF EXISTS `os_groups_roles`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `os_groups_roles` (
  `GroupID` char(36) NOT NULL DEFAULT '',
  `RoleID` char(36) NOT NULL DEFAULT '',
  `Name` varchar(255) CHARACTER SET utf8mb3 COLLATE utf8mb3_general_ci NOT NULL DEFAULT '',
  `Description` varchar(255) CHARACTER SET utf8mb3 COLLATE utf8mb3_general_ci NOT NULL DEFAULT '',
  `Title` varchar(255) CHARACTER SET utf8mb3 COLLATE utf8mb3_general_ci NOT NULL DEFAULT '',
  `Powers` bigint unsigned NOT NULL DEFAULT '0',
  PRIMARY KEY (`GroupID`,`RoleID`),
  KEY `GroupID` (`GroupID`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `os_groups_roles` DISABLE KEYS */;
/*!40000 ALTER TABLE `os_groups_roles` ENABLE KEYS */;
DROP TABLE IF EXISTS `primitems`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `primitems` (
  `invType` int DEFAULT NULL,
  `assetType` int DEFAULT NULL,
  `name` varchar(255) DEFAULT NULL,
  `description` varchar(255) DEFAULT NULL,
  `creationDate` bigint DEFAULT NULL,
  `nextPermissions` int DEFAULT NULL,
  `currentPermissions` int DEFAULT NULL,
  `basePermissions` int DEFAULT NULL,
  `everyonePermissions` int DEFAULT NULL,
  `groupPermissions` int DEFAULT NULL,
  `flags` int NOT NULL DEFAULT '0',
  `itemID` char(36) NOT NULL DEFAULT '',
  `primID` char(36) DEFAULT NULL,
  `assetID` char(36) DEFAULT NULL,
  `parentFolderID` char(36) DEFAULT NULL,
  `CreatorID` varchar(255) NOT NULL DEFAULT '',
  `ownerID` char(36) DEFAULT NULL,
  `groupID` char(36) DEFAULT NULL,
  `lastOwnerID` char(36) DEFAULT NULL,
  `experienceID` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  PRIMARY KEY (`itemID`),
  KEY `primitems_primid` (`primID`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `primitems` DISABLE KEYS */;
/*!40000 ALTER TABLE `primitems` ENABLE KEYS */;
DROP TABLE IF EXISTS `prims`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `prims` (
  `CreationDate` int DEFAULT NULL,
  `Name` varchar(255) CHARACTER SET utf8mb3 COLLATE utf8mb3_general_ci DEFAULT NULL,
  `Text` varchar(255) CHARACTER SET utf8mb3 COLLATE utf8mb3_general_ci DEFAULT NULL,
  `Description` varchar(255) CHARACTER SET utf8mb3 COLLATE utf8mb3_general_ci DEFAULT NULL,
  `SitName` varchar(255) CHARACTER SET utf8mb3 COLLATE utf8mb3_general_ci DEFAULT NULL,
  `TouchName` varchar(255) CHARACTER SET utf8mb3 COLLATE utf8mb3_general_ci DEFAULT NULL,
  `ObjectFlags` int DEFAULT NULL,
  `OwnerMask` int DEFAULT NULL,
  `NextOwnerMask` int DEFAULT NULL,
  `GroupMask` int DEFAULT NULL,
  `EveryoneMask` int DEFAULT NULL,
  `BaseMask` int DEFAULT NULL,
  `PositionX` float DEFAULT '0',
  `PositionY` float DEFAULT '0',
  `PositionZ` float DEFAULT '0',
  `GroupPositionX` float DEFAULT '0',
  `GroupPositionY` float DEFAULT '0',
  `GroupPositionZ` float DEFAULT '0',
  `VelocityX` float DEFAULT '0',
  `VelocityY` float DEFAULT '0',
  `VelocityZ` float DEFAULT '0',
  `AngularVelocityX` float DEFAULT '0',
  `AngularVelocityY` float DEFAULT '0',
  `AngularVelocityZ` float DEFAULT '0',
  `AccelerationX` float DEFAULT '0',
  `AccelerationY` float DEFAULT '0',
  `AccelerationZ` float DEFAULT '0',
  `RotationX` float DEFAULT '0',
  `RotationY` float DEFAULT '0',
  `RotationZ` float DEFAULT '0',
  `RotationW` float DEFAULT '0',
  `SitTargetOffsetX` float DEFAULT '0',
  `SitTargetOffsetY` float DEFAULT '0',
  `SitTargetOffsetZ` float DEFAULT '0',
  `SitTargetOrientW` float DEFAULT '0',
  `SitTargetOrientX` float DEFAULT '0',
  `SitTargetOrientY` float DEFAULT '0',
  `SitTargetOrientZ` float DEFAULT '0',
  `UUID` char(36) NOT NULL DEFAULT '',
  `RegionUUID` char(36) DEFAULT NULL,
  `CreatorID` varchar(255) NOT NULL DEFAULT '',
  `OwnerID` char(36) DEFAULT NULL,
  `GroupID` char(36) DEFAULT NULL,
  `LastOwnerID` char(36) DEFAULT NULL,
  `SceneGroupID` char(36) DEFAULT NULL,
  `PayPrice` int NOT NULL DEFAULT '0',
  `PayButton1` int NOT NULL DEFAULT '0',
  `PayButton2` int NOT NULL DEFAULT '0',
  `PayButton3` int NOT NULL DEFAULT '0',
  `PayButton4` int NOT NULL DEFAULT '0',
  `LoopedSound` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `LoopedSoundGain` float DEFAULT '0',
  `TextureAnimation` blob,
  `OmegaX` float DEFAULT '0',
  `OmegaY` float DEFAULT '0',
  `OmegaZ` float DEFAULT '0',
  `CameraEyeOffsetX` float DEFAULT '0',
  `CameraEyeOffsetY` float DEFAULT '0',
  `CameraEyeOffsetZ` float DEFAULT '0',
  `CameraAtOffsetX` float DEFAULT '0',
  `CameraAtOffsetY` float DEFAULT '0',
  `CameraAtOffsetZ` float DEFAULT '0',
  `ForceMouselook` tinyint NOT NULL DEFAULT '0',
  `ScriptAccessPin` int NOT NULL DEFAULT '0',
  `AllowedDrop` tinyint NOT NULL DEFAULT '0',
  `DieAtEdge` tinyint NOT NULL DEFAULT '0',
  `SalePrice` int NOT NULL DEFAULT '10',
  `SaleType` tinyint NOT NULL DEFAULT '0',
  `ColorR` int NOT NULL DEFAULT '0',
  `ColorG` int NOT NULL DEFAULT '0',
  `ColorB` int NOT NULL DEFAULT '0',
  `ColorA` int NOT NULL DEFAULT '0',
  `ParticleSystem` blob,
  `ClickAction` tinyint NOT NULL DEFAULT '0',
  `Material` tinyint NOT NULL DEFAULT '3',
  `CollisionSound` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `CollisionSoundVolume` double NOT NULL DEFAULT '0',
  `LinkNumber` int NOT NULL DEFAULT '0',
  `PassTouches` tinyint NOT NULL DEFAULT '0',
  `MediaURL` varchar(255) DEFAULT NULL,
  `DynAttrs` text,
  `PhysicsShapeType` tinyint NOT NULL DEFAULT '0',
  `Density` float DEFAULT '1000',
  `GravityModifier` float DEFAULT '1',
  `Friction` float DEFAULT '0.6',
  `Restitution` float DEFAULT '0.5',
  `KeyframeMotion` blob,
  `AttachedPosX` float DEFAULT '0',
  `AttachedPosY` float DEFAULT '0',
  `AttachedPosZ` float DEFAULT '0',
  `PassCollisions` tinyint NOT NULL DEFAULT '0',
  `Vehicle` text,
  `RotationAxisLocks` tinyint NOT NULL DEFAULT '0',
  `RezzerID` char(36) DEFAULT NULL,
  `PhysInertia` text,
  `sopanims` blob,
  `standtargetx` float DEFAULT '0',
  `standtargety` float DEFAULT '0',
  `standtargetz` float DEFAULT '0',
  `sitactrange` float DEFAULT '0',
  `pseudocrc` int DEFAULT '0',
  `linksetdata` mediumtext,
  `AllowUnsit` tinyint DEFAULT '1',
  `ScriptedSitOnly` tinyint DEFAULT '0',
  `StartStr` text,
  `SitTargetActive` tinyint DEFAULT NULL,
  PRIMARY KEY (`UUID`),
  KEY `prims_regionuuid` (`RegionUUID`),
  KEY `prims_scenegroupid` (`SceneGroupID`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `prims` DISABLE KEYS */;
/*!40000 ALTER TABLE `prims` ENABLE KEYS */;
DROP TABLE IF EXISTS `primshapes`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `primshapes` (
  `Shape` int DEFAULT NULL,
  `ScaleX` double NOT NULL DEFAULT '0',
  `ScaleY` double NOT NULL DEFAULT '0',
  `ScaleZ` double NOT NULL DEFAULT '0',
  `PCode` int DEFAULT NULL,
  `PathBegin` int DEFAULT NULL,
  `PathEnd` int DEFAULT NULL,
  `PathScaleX` int DEFAULT NULL,
  `PathScaleY` int DEFAULT NULL,
  `PathShearX` int DEFAULT NULL,
  `PathShearY` int DEFAULT NULL,
  `PathSkew` int DEFAULT NULL,
  `PathCurve` int DEFAULT NULL,
  `PathRadiusOffset` int DEFAULT NULL,
  `PathRevolutions` int DEFAULT NULL,
  `PathTaperX` int DEFAULT NULL,
  `PathTaperY` int DEFAULT NULL,
  `PathTwist` int DEFAULT NULL,
  `PathTwistBegin` int DEFAULT NULL,
  `ProfileBegin` int DEFAULT NULL,
  `ProfileEnd` int DEFAULT NULL,
  `ProfileCurve` int DEFAULT NULL,
  `ProfileHollow` int DEFAULT NULL,
  `State` int DEFAULT NULL,
  `Texture` longblob,
  `ExtraParams` longblob,
  `UUID` char(36) NOT NULL DEFAULT '',
  `Media` text,
  `LastAttachPoint` int NOT NULL DEFAULT '0',
  `MatOvrd` blob,
  PRIMARY KEY (`UUID`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `primshapes` DISABLE KEYS */;
/*!40000 ALTER TABLE `primshapes` ENABLE KEYS */;
DROP TABLE IF EXISTS `regionban`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `regionban` (
  `regionUUID` varchar(36) NOT NULL,
  `bannedUUID` varchar(36) NOT NULL,
  `bannedIp` varchar(16) NOT NULL,
  `bannedIpHostMask` varchar(16) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `regionban` DISABLE KEYS */;
/*!40000 ALTER TABLE `regionban` ENABLE KEYS */;
DROP TABLE IF EXISTS `regionenvironment`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `regionenvironment` (
  `region_id` varchar(36) NOT NULL,
  `llsd_settings` mediumtext,
  PRIMARY KEY (`region_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `regionenvironment` DISABLE KEYS */;
/*!40000 ALTER TABLE `regionenvironment` ENABLE KEYS */;
DROP TABLE IF EXISTS `regionextra`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `regionextra` (
  `RegionID` char(36) NOT NULL,
  `Name` varchar(32) NOT NULL,
  `value` text,
  PRIMARY KEY (`RegionID`,`Name`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `regionextra` DISABLE KEYS */;
/*!40000 ALTER TABLE `regionextra` ENABLE KEYS */;
DROP TABLE IF EXISTS `regions`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `regions` (
  `uuid` varchar(36) NOT NULL,
  `regionHandle` bigint unsigned NOT NULL,
  `regionName` varchar(128) DEFAULT NULL,
  `regionRecvKey` varchar(128) DEFAULT NULL,
  `regionSendKey` varchar(128) DEFAULT NULL,
  `regionSecret` varchar(128) DEFAULT NULL,
  `regionDataURI` varchar(255) DEFAULT NULL,
  `serverIP` varchar(64) DEFAULT NULL,
  `serverPort` int unsigned DEFAULT NULL,
  `serverURI` varchar(255) DEFAULT NULL,
  `locX` int unsigned DEFAULT NULL,
  `locY` int unsigned DEFAULT NULL,
  `locZ` int unsigned DEFAULT NULL,
  `eastOverrideHandle` bigint unsigned DEFAULT NULL,
  `westOverrideHandle` bigint unsigned DEFAULT NULL,
  `southOverrideHandle` bigint unsigned DEFAULT NULL,
  `northOverrideHandle` bigint unsigned DEFAULT NULL,
  `regionAssetURI` varchar(255) DEFAULT NULL,
  `regionAssetRecvKey` varchar(128) DEFAULT NULL,
  `regionAssetSendKey` varchar(128) DEFAULT NULL,
  `regionUserURI` varchar(255) DEFAULT NULL,
  `regionUserRecvKey` varchar(128) DEFAULT NULL,
  `regionUserSendKey` varchar(128) DEFAULT NULL,
  `regionMapTexture` varchar(36) DEFAULT NULL,
  `serverHttpPort` int DEFAULT NULL,
  `serverRemotingPort` int DEFAULT NULL,
  `owner_uuid` varchar(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `originUUID` varchar(36) DEFAULT NULL,
  `access` int unsigned DEFAULT '1',
  `ScopeID` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `sizeX` int NOT NULL DEFAULT '0',
  `sizeY` int NOT NULL DEFAULT '0',
  `flags` int NOT NULL DEFAULT '0',
  `last_seen` int NOT NULL DEFAULT '0',
  `PrincipalID` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `Token` varchar(255) NOT NULL,
  `parcelMapTexture` varchar(36) DEFAULT NULL,
  `viewerLocalAddress` varchar(64) DEFAULT NULL,
  PRIMARY KEY (`uuid`),
  KEY `regionName` (`regionName`),
  KEY `regionHandle` (`regionHandle`),
  KEY `overrideHandles` (`eastOverrideHandle`,`westOverrideHandle`,`southOverrideHandle`,`northOverrideHandle`),
  KEY `ScopeID` (`ScopeID`),
  KEY `flags` (`flags`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `regions` DISABLE KEYS */;
/*!40000 ALTER TABLE `regions` ENABLE KEYS */;
DROP TABLE IF EXISTS `regionsettings`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `regionsettings` (
  `regionUUID` char(36) NOT NULL,
  `block_terraform` int NOT NULL,
  `block_fly` int NOT NULL,
  `allow_damage` int NOT NULL,
  `restrict_pushing` int NOT NULL,
  `allow_land_resell` int NOT NULL,
  `allow_land_join_divide` int NOT NULL,
  `block_show_in_search` int NOT NULL,
  `agent_limit` int NOT NULL,
  `object_bonus` double NOT NULL,
  `maturity` int NOT NULL,
  `disable_scripts` int NOT NULL,
  `disable_collisions` int NOT NULL,
  `disable_physics` int NOT NULL,
  `terrain_texture_1` char(36) NOT NULL,
  `terrain_texture_2` char(36) NOT NULL,
  `terrain_texture_3` char(36) NOT NULL,
  `terrain_texture_4` char(36) NOT NULL,
  `elevation_1_nw` double NOT NULL,
  `elevation_2_nw` double NOT NULL,
  `elevation_1_ne` double NOT NULL,
  `elevation_2_ne` double NOT NULL,
  `elevation_1_se` double NOT NULL,
  `elevation_2_se` double NOT NULL,
  `elevation_1_sw` double NOT NULL,
  `elevation_2_sw` double NOT NULL,
  `water_height` double NOT NULL,
  `terrain_raise_limit` double NOT NULL,
  `terrain_lower_limit` double NOT NULL,
  `use_estate_sun` int NOT NULL,
  `fixed_sun` int NOT NULL,
  `sun_position` double NOT NULL,
  `covenant` char(36) DEFAULT NULL,
  `Sandbox` tinyint NOT NULL,
  `sunvectorx` double NOT NULL DEFAULT '0',
  `sunvectory` double NOT NULL DEFAULT '0',
  `sunvectorz` double NOT NULL DEFAULT '0',
  `loaded_creation_id` varchar(64) DEFAULT NULL,
  `loaded_creation_datetime` int unsigned NOT NULL DEFAULT '0',
  `map_tile_ID` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `TelehubObject` varchar(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `parcel_tile_ID` char(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `covenant_datetime` int unsigned NOT NULL DEFAULT '0',
  `block_search` tinyint NOT NULL DEFAULT '0',
  `casino` tinyint NOT NULL DEFAULT '0',
  `cacheID` char(36) DEFAULT NULL,
  `TerrainPBR1` varchar(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `TerrainPBR2` varchar(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `TerrainPBR3` varchar(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  `TerrainPBR4` varchar(36) NOT NULL DEFAULT '00000000-0000-0000-0000-000000000000',
  PRIMARY KEY (`regionUUID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `regionsettings` DISABLE KEYS */;
/*!40000 ALTER TABLE `regionsettings` ENABLE KEYS */;
DROP TABLE IF EXISTS `regionwindlight`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `regionwindlight` (
  `region_id` varchar(36) NOT NULL DEFAULT '000000-0000-0000-0000-000000000000',
  `water_color_r` float(9,6) unsigned NOT NULL DEFAULT '4.000000',
  `water_color_g` float(9,6) unsigned NOT NULL DEFAULT '38.000000',
  `water_color_b` float(9,6) unsigned NOT NULL DEFAULT '64.000000',
  `water_fog_density_exponent` float(9,7) unsigned NOT NULL DEFAULT '4.0000000',
  `underwater_fog_modifier` float(9,8) unsigned NOT NULL DEFAULT '0.25000000',
  `reflection_wavelet_scale_1` float(9,7) unsigned NOT NULL DEFAULT '2.0000000',
  `reflection_wavelet_scale_2` float(9,7) unsigned NOT NULL DEFAULT '2.0000000',
  `reflection_wavelet_scale_3` float(9,7) unsigned NOT NULL DEFAULT '2.0000000',
  `fresnel_scale` float(9,8) unsigned NOT NULL DEFAULT '0.40000001',
  `fresnel_offset` float(9,8) unsigned NOT NULL DEFAULT '0.50000000',
  `refract_scale_above` float(9,8) unsigned NOT NULL DEFAULT '0.03000000',
  `refract_scale_below` float(9,8) unsigned NOT NULL DEFAULT '0.20000000',
  `blur_multiplier` float(9,8) unsigned NOT NULL DEFAULT '0.04000000',
  `big_wave_direction_x` float(9,8) NOT NULL DEFAULT '1.04999995',
  `big_wave_direction_y` float(9,8) NOT NULL DEFAULT '-0.41999999',
  `little_wave_direction_x` float(9,8) NOT NULL DEFAULT '1.11000001',
  `little_wave_direction_y` float(9,8) NOT NULL DEFAULT '-1.15999997',
  `normal_map_texture` varchar(36) NOT NULL DEFAULT '822ded49-9a6c-f61c-cb89-6df54f42cdf4',
  `horizon_r` float(9,8) unsigned NOT NULL DEFAULT '0.25000000',
  `horizon_g` float(9,8) unsigned NOT NULL DEFAULT '0.25000000',
  `horizon_b` float(9,8) unsigned NOT NULL DEFAULT '0.31999999',
  `horizon_i` float(9,8) unsigned NOT NULL DEFAULT '0.31999999',
  `haze_horizon` float(9,8) unsigned NOT NULL DEFAULT '0.19000000',
  `blue_density_r` float(9,8) unsigned NOT NULL DEFAULT '0.12000000',
  `blue_density_g` float(9,8) unsigned NOT NULL DEFAULT '0.22000000',
  `blue_density_b` float(9,8) unsigned NOT NULL DEFAULT '0.38000000',
  `blue_density_i` float(9,8) unsigned NOT NULL DEFAULT '0.38000000',
  `haze_density` float(9,8) unsigned NOT NULL DEFAULT '0.69999999',
  `density_multiplier` float(9,8) unsigned NOT NULL DEFAULT '0.18000001',
  `distance_multiplier` float(9,6) unsigned NOT NULL DEFAULT '0.800000',
  `max_altitude` int unsigned NOT NULL DEFAULT '1605',
  `sun_moon_color_r` float(9,8) unsigned NOT NULL DEFAULT '0.23999999',
  `sun_moon_color_g` float(9,8) unsigned NOT NULL DEFAULT '0.25999999',
  `sun_moon_color_b` float(9,8) unsigned NOT NULL DEFAULT '0.30000001',
  `sun_moon_color_i` float(9,8) unsigned NOT NULL DEFAULT '0.30000001',
  `sun_moon_position` float(9,8) unsigned NOT NULL DEFAULT '0.31700000',
  `ambient_r` float(9,8) unsigned NOT NULL DEFAULT '0.34999999',
  `ambient_g` float(9,8) unsigned NOT NULL DEFAULT '0.34999999',
  `ambient_b` float(9,8) unsigned NOT NULL DEFAULT '0.34999999',
  `ambient_i` float(9,8) unsigned NOT NULL DEFAULT '0.34999999',
  `east_angle` float(9,8) unsigned NOT NULL DEFAULT '0.00000000',
  `sun_glow_focus` float(9,8) unsigned NOT NULL DEFAULT '0.10000000',
  `sun_glow_size` float(9,8) unsigned NOT NULL DEFAULT '1.75000000',
  `scene_gamma` float(9,7) unsigned NOT NULL DEFAULT '1.0000000',
  `star_brightness` float(9,8) unsigned NOT NULL DEFAULT '0.00000000',
  `cloud_color_r` float(9,8) unsigned NOT NULL DEFAULT '0.41000000',
  `cloud_color_g` float(9,8) unsigned NOT NULL DEFAULT '0.41000000',
  `cloud_color_b` float(9,8) unsigned NOT NULL DEFAULT '0.41000000',
  `cloud_color_i` float(9,8) unsigned NOT NULL DEFAULT '0.41000000',
  `cloud_x` float(9,8) unsigned NOT NULL DEFAULT '1.00000000',
  `cloud_y` float(9,8) unsigned NOT NULL DEFAULT '0.52999997',
  `cloud_density` float(9,8) unsigned NOT NULL DEFAULT '1.00000000',
  `cloud_coverage` float(9,8) unsigned NOT NULL DEFAULT '0.27000001',
  `cloud_scale` float(9,8) unsigned NOT NULL DEFAULT '0.41999999',
  `cloud_detail_x` float(9,8) unsigned NOT NULL DEFAULT '1.00000000',
  `cloud_detail_y` float(9,8) unsigned NOT NULL DEFAULT '0.52999997',
  `cloud_detail_density` float(9,8) unsigned NOT NULL DEFAULT '0.12000000',
  `cloud_scroll_x` float(9,7) NOT NULL DEFAULT '0.2000000',
  `cloud_scroll_x_lock` tinyint unsigned NOT NULL DEFAULT '0',
  `cloud_scroll_y` float(9,7) NOT NULL DEFAULT '0.0100000',
  `cloud_scroll_y_lock` tinyint unsigned NOT NULL DEFAULT '0',
  `draw_classic_clouds` tinyint unsigned NOT NULL DEFAULT '1',
  PRIMARY KEY (`region_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `regionwindlight` DISABLE KEYS */;
/*!40000 ALTER TABLE `regionwindlight` ENABLE KEYS */;
DROP TABLE IF EXISTS `spawn_points`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `spawn_points` (
  `RegionID` varchar(36) CHARACTER SET utf8mb3 COLLATE utf8mb3_unicode_ci NOT NULL,
  `Yaw` float NOT NULL,
  `Pitch` float NOT NULL,
  `Distance` float NOT NULL,
  KEY `RegionID` (`RegionID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `spawn_points` DISABLE KEYS */;
/*!40000 ALTER TABLE `spawn_points` ENABLE KEYS */;
DROP TABLE IF EXISTS `terrain`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `terrain` (
  `RegionUUID` varchar(255) DEFAULT NULL,
  `Revision` int DEFAULT NULL,
  `Heightfield` longblob
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `terrain` DISABLE KEYS */;
/*!40000 ALTER TABLE `terrain` ENABLE KEYS */;
DROP TABLE IF EXISTS `tokens`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `tokens` (
  `UUID` char(36) NOT NULL,
  `token` varchar(255) NOT NULL,
  `validity` datetime NOT NULL,
  UNIQUE KEY `uuid_token` (`UUID`,`token`),
  KEY `UUID` (`UUID`),
  KEY `token` (`token`),
  KEY `validity` (`validity`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb3;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `tokens` DISABLE KEYS */;
/*!40000 ALTER TABLE `tokens` ENABLE KEYS */;
DROP TABLE IF EXISTS `userdata`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `userdata` (
  `UserId` char(36) NOT NULL,
  `TagId` varchar(64) NOT NULL,
  `DataKey` varchar(255) DEFAULT NULL,
  `DataVal` varchar(255) DEFAULT NULL,
  PRIMARY KEY (`UserId`,`TagId`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `userdata` DISABLE KEYS */;
/*!40000 ALTER TABLE `userdata` ENABLE KEYS */;
DROP TABLE IF EXISTS `usernotes`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `usernotes` (
  `useruuid` varchar(36) NOT NULL,
  `targetuuid` varchar(36) NOT NULL,
  `notes` text NOT NULL,
  UNIQUE KEY `useruuid` (`useruuid`,`targetuuid`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `usernotes` DISABLE KEYS */;
/*!40000 ALTER TABLE `usernotes` ENABLE KEYS */;
DROP TABLE IF EXISTS `userpicks`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `userpicks` (
  `pickuuid` varchar(36) NOT NULL,
  `creatoruuid` varchar(36) NOT NULL,
  `toppick` enum('true','false') NOT NULL,
  `parceluuid` varchar(36) NOT NULL,
  `name` varchar(255) NOT NULL,
  `description` text NOT NULL,
  `snapshotuuid` varchar(36) NOT NULL,
  `user` varchar(255) NOT NULL,
  `originalname` varchar(255) NOT NULL,
  `simname` varchar(255) NOT NULL,
  `posglobal` varchar(255) NOT NULL,
  `sortorder` int NOT NULL,
  `enabled` enum('true','false') NOT NULL,
  `gatekeeper` varchar(255) DEFAULT NULL,
  PRIMARY KEY (`pickuuid`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `userpicks` DISABLE KEYS */;
/*!40000 ALTER TABLE `userpicks` ENABLE KEYS */;
DROP TABLE IF EXISTS `userprofile`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `userprofile` (
  `useruuid` varchar(36) NOT NULL,
  `profilePartner` varchar(36) NOT NULL,
  `profileAllowPublish` binary(1) NOT NULL,
  `profileMaturePublish` binary(1) NOT NULL,
  `profileURL` varchar(255) NOT NULL,
  `profileWantToMask` int NOT NULL,
  `profileWantToText` text NOT NULL,
  `profileSkillsMask` int NOT NULL,
  `profileSkillsText` text NOT NULL,
  `profileLanguages` text NOT NULL,
  `profileImage` varchar(36) NOT NULL,
  `profileAboutText` text NOT NULL,
  `profileFirstImage` varchar(36) NOT NULL,
  `profileFirstText` text NOT NULL,
  PRIMARY KEY (`useruuid`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `userprofile` DISABLE KEYS */;
/*!40000 ALTER TABLE `userprofile` ENABLE KEYS */;
DROP TABLE IF EXISTS `usersettings`;
/*!40101 SET @saved_cs_client     = @@character_set_client */;
/*!50503 SET character_set_client = utf8mb4 */;
CREATE TABLE `usersettings` (
  `useruuid` varchar(36) NOT NULL,
  `imviaemail` enum('true','false') NOT NULL,
  `visible` enum('true','false') NOT NULL,
  `email` varchar(254) NOT NULL,
  PRIMARY KEY (`useruuid`)
) ENGINE=InnoDB DEFAULT CHARSET=latin1;
/*!40101 SET character_set_client = @saved_cs_client */;

/*!40000 ALTER TABLE `usersettings` DISABLE KEYS */;
/*!40000 ALTER TABLE `usersettings` ENABLE KEYS */;
/*!40103 SET TIME_ZONE=@OLD_TIME_ZONE */;

/*!40101 SET SQL_MODE=@OLD_SQL_MODE */;
/*!40014 SET FOREIGN_KEY_CHECKS=@OLD_FOREIGN_KEY_CHECKS */;
/*!40014 SET UNIQUE_CHECKS=@OLD_UNIQUE_CHECKS */;
/*!40101 SET CHARACTER_SET_CLIENT=@OLD_CHARACTER_SET_CLIENT */;
/*!40101 SET CHARACTER_SET_RESULTS=@OLD_CHARACTER_SET_RESULTS */;
/*!40101 SET COLLATION_CONNECTION=@OLD_COLLATION_CONNECTION */;
/*!40111 SET SQL_NOTES=@OLD_SQL_NOTES */;

