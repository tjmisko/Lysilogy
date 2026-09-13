CREATE VIRTUAL TABLE work_search USING fts5(id UNINDEXED, title, tokenize='trigram');
CREATE VIRTUAL TABLE person_search USING fts5(id UNINDEXED, name, tokenize='trigram');
CREATE TRIGGER work_insert AFTER INSERT ON works BEGIN
    INSERT INTO work_search(rowid, id, title) VALUES(new.rowid, new.id, new.title_key);
END;
CREATE TRIGGER work_delete AFTER DELETE ON works BEGIN
    DELETE FROM work_search WHERE rowid = old.rowid;
END;
CREATE TRIGGER work_update AFTER UPDATE ON works BEGIN
    DELETE FROM work_search WHERE rowid = old.rowid;
    INSERT INTO work_search(rowid, id, title) VALUES(new.rowid, new.id, new.title_key);
END;
CREATE TRIGGER person_insert AFTER INSERT ON persons BEGIN
    INSERT INTO person_search(rowid, id, name) VALUES(new.rowid, new.id, new.name);
END;
CREATE TRIGGER person_delete AFTER DELETE ON persons BEGIN
    DELETE FROM person_search WHERE rowid = old.rowid;
END;
CREATE TRIGGER person_update AFTER UPDATE ON persons BEGIN
    DELETE FROM person_search WHERE rowid = old.rowid;
    INSERT INTO person_search(rowid, id, name) VALUES(new.rowid, new.id, new.name);
END;
INSERT INTO work_search(rowid, id, title) SELECT rowid, id, title_key FROM works;
INSERT INTO person_search(rowid, id, name) SELECT rowid, id, name FROM persons;
