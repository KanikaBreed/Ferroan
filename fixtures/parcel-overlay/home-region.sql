-- The live harness home region. Leave the Q60a seed's other region intact.
UPDATE land SET RegionUUID='6d1a7f4e-0c3b-4f53-9a1e-5f0e3c2a9b01'
WHERE RegionUUID='11111111-1111-1111-1111-111111111111';
-- Pin8 rewrites access lists at startup. Repeated parcel/avatar pairs abort
-- that save (TB128). Access entries do not participate in ParcelOverlay.
DELETE FROM landaccesslist WHERE rowid NOT IN
 (SELECT min(rowid) FROM landaccesslist GROUP BY LandUUID, AccessUUID);
