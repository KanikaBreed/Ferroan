-- Q60a uses a copy of objects.db (Pin8 RegionStore schema).
INSERT INTO land
(UUID, RegionUUID, LocalLandID, Bitmap, Name, Desc, OwnerUUID, IsGroupOwned,
 Area, AuctionID, Category, ClaimDate, ClaimPrice, GroupUUID, SalePrice, LandStatus,
 LandFlags, LandingType, MediaAutoScale, MediaTextureUUID, MediaURL, MusicURL,
 PassHours, PassPrice, SnapshotUUID, UserLocationX, UserLocationY, UserLocationZ,
 UserLookAtX, UserLookAtY, UserLookAtZ, AuthbuyerID, OtherCleanTime, Dwell,
 MediaType, MediaDescription, MediaSize, MediaLoop, ObscureMedia, ObscureMusic,
 SeeAVs, AnyAVSounds, GroupAVSounds, environment)
VALUES
('aaaaaaaa-0000-0000-0000-000000000001','11111111-1111-1111-1111-111111111111',
 91,X'8201','West','First parcel','00000000-0000-0000-0000-000000000001','False',
 48,7,3,1700000000,21,'00000000-0000-0000-0000-000000000003',41,1,
 2147516420,1,1,'00000000-0000-0000-0000-000000000004','https://example.invalid/media',
 'https://example.invalid/music',1.5,8,'00000000-0000-0000-0000-000000000005',
 1.25,2.5,3.75,-1,0,1,'00000000-0000-0000-0000-000000000006',12,17,
 'video/test','Screen','640,320',1,0,1,0,1,0,'<? llsd/notation ?>{"env_version":i7}');

CREATE TEMP TABLE parcel_copy AS SELECT * FROM land WHERE Name = 'West';
UPDATE parcel_copy SET UUID='99999999-0000-0000-0000-000000000002', LocalLandID=2,
 Bitmap=X'04', Name='East', OwnerUUID=GroupUUID, IsGroupOwned='True', environment=NULL,
 AuthbuyerID='invalid', LandStatus=255, Category=257, MediaSize='unset', UserLookAtY=NULL;
INSERT INTO land SELECT * FROM parcel_copy;
UPDATE parcel_copy SET UUID='cccccccc-0000-0000-0000-000000000003',
 RegionUUID='22222222-2222-2222-2222-222222222222', Name='Other region';
INSERT INTO land SELECT * FROM parcel_copy;
DROP TABLE parcel_copy;

INSERT INTO landaccesslist (LandUUID, AccessUUID, Flags) VALUES
 ('99999999-0000-0000-0000-000000000002','00000000-0000-0000-0000-000000000021',2),
 ('aaaaaaaa-0000-0000-0000-000000000001','00000000-0000-0000-0000-000000000011',1),
 ('cccccccc-0000-0000-0000-000000000003','00000000-0000-0000-0000-000000000031',3),
 ('aaaaaaaa-0000-0000-0000-000000000001','00000000-0000-0000-0000-000000000012',2),
 ('aaaaaaaa-0000-0000-0000-000000000001','00000000-0000-0000-0000-000000000011',1),
 ('aaaaaaaa-0000-0000-0000-000000000001','00000000-0000-0000-0000-000000000011',2);
