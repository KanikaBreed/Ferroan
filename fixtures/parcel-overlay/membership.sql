-- The second viewer belongs to East's group, with no active group.
INSERT INTO os_groups_groups (GroupID,Name,FounderID,InsigniaID,OwnerRoleID)
VALUES ('00000000-0000-0000-0000-000000000003','Test parcel group',
 '8c4e1a70-2b59-4d6f-9e13-5a7c0d8b2f44','00000000-0000-0000-0000-000000000000','00000000-0000-0000-0000-000000000009');
INSERT INTO os_groups_roles (GroupID,RoleID,Name,Title) VALUES
 ('00000000-0000-0000-0000-000000000003','00000000-0000-0000-0000-000000000000','Everyone','Member');
INSERT INTO os_groups_membership (GroupID,PrincipalID,SelectedRoleID)
SELECT GroupID,FounderID,InsigniaID FROM os_groups_groups WHERE Name='Test parcel group';
INSERT INTO os_groups_rolemembership (GroupID,RoleID,PrincipalID)
SELECT GroupID,InsigniaID,FounderID FROM os_groups_groups WHERE Name='Test parcel group';
