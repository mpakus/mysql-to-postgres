-- Independently authored deterministic fixtures. No upstream SQL copied.
SET SESSION time_zone = '+00:00';
SET NAMES utf8mb4;
USE source;
CREATE TABLE users (
  id INT NOT NULL AUTO_INCREMENT PRIMARY KEY,
  name VARCHAR(100) NOT NULL,
  note TEXT NULL,
  payload BLOB NULL,
  amount DECIMAL(30,8) NOT NULL,
  created_at DATETIME(6) NOT NULL
) ENGINE=InnoDB;
INSERT INTO users VALUES
  (1, 'Alice', NULL, X'00015C09FF', 1234567890123456789012.12345678, '2024-01-02 03:04:05.123456'),
  (4, 'emoji 😀', '', X'', -0.00000001, '2024-07-08 09:10:11.000001'),
  (9, 'escapes', CONCAT('literal \\N', CHAR(9), 'tab', CHAR(10), 'line', CHAR(13), 'return'), NULL, 0.00000000, '2000-02-29 12:00:00.000000');
ALTER TABLE users AUTO_INCREMENT = 42;
CREATE TABLE orders (
  user_id INT NOT NULL,
  order_id INT NOT NULL,
  label VARCHAR(40) NOT NULL,
  PRIMARY KEY(user_id, order_id),
  UNIQUE KEY order_label (label),
  CONSTRAINT orders_user FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE
) ENGINE=InnoDB;
INSERT INTO orders VALUES (1, 2, 'first'), (4, 1, 'second');
CREATE TABLE empty_table (id INT NOT NULL AUTO_INCREMENT PRIMARY KEY, value VARCHAR(30) DEFAULT '') ENGINE=InnoDB;
CREATE TABLE keyless (value VARCHAR(50), qty INT) ENGINE=InnoDB;
INSERT INTO keyless VALUES ('same', 2), ('same', 2), ('same', 3), (NULL, 0), ('', 0);
CREATE TABLE `CamelCase` (`MixedColumn` INT, `select` VARCHAR(20)) ENGINE=InnoDB;
INSERT INTO `CamelCase` VALUES (7, 'quoted');
CREATE TABLE numeric_edges (
  id INT PRIMARY KEY,
  signed_big BIGINT,
  unsigned_big BIGINT UNSIGNED,
  unsigned_int INT UNSIGNED,
  tiny_one TINYINT(1),
  bits BIT(8),
  duration TIME(6),
  value_json JSON
) ENGINE=InnoDB;
INSERT INTO numeric_edges VALUES
  (1, -9223372036854775808, 18446744073709551615, 4294967295, 2, b'00000101', '-838:59:58.999999', '{"large":18446744073709551615,"null":null}'),
  (2, 9223372036854775807, 9223372036854775808, 0, -1, b'11111111', '48:01:02.123456', 'null');
CREATE TABLE all_bytes (id INT PRIMARY KEY, value BLOB) ENGINE=InnoDB;
-- Filled by the harness using exact hex of bytes 00..ff.
CREATE TABLE invalid_values (id INT PRIMARY KEY, invalid_date DATE, nul_text TEXT) ENGINE=InnoDB;
SET SESSION sql_mode = '';
INSERT INTO invalid_values VALUES (1, '0000-00-00', CONCAT('before', CHAR(0), 'after'));
CREATE VIEW existing_view AS SELECT id, name FROM users;
