DELETE FROM land WHERE UUID='99999999-0000-0000-0000-000000000002';
-- West remains for sale, at no cost, exclusively to the second viewer.
UPDATE land SET SalePrice=0, AuthbuyerID='8c4e1a70-2b59-4d6f-9e13-5a7c0d8b2f44'
WHERE UUID='aaaaaaaa-0000-0000-0000-000000000001';
