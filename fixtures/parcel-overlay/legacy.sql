-- A 64-cell legacy row in a 128-cell-wide region. The second bit run
-- must land on row one, across a different border from the first.
UPDATE land SET Bitmap=CAST(X'02' || zeroblob(7) || X'18' || zeroblob(503) AS BLOB),
 OwnerUUID='00000000-0000-0000-0000-000000000000', LandFlags=32768, SeeAVs=1
WHERE UUID='aaaaaaaa-0000-0000-0000-000000000001';
-- Distinguish local sound on West from hidden avatars on East.
UPDATE land SET LandFlags=4, SeeAVs=0
WHERE UUID='99999999-0000-0000-0000-000000000002';
