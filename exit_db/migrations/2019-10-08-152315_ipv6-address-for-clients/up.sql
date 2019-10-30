ALTER TABLE clients RENAME TO clients_old;

CREATE TABLE clients
(
    mesh_ip varchar(40) PRIMARY KEY,
    wg_pubkey varchar(44) NOT NULL,
    wg_port integer NOT NULL,
    eth_address varchar(64) NOT NULL,
    internal_ip varchar(42) NOT NULL,
    internal_ipv6 varchar(128) NOT NULL,
    nickname varchar(32) NOT NULL,
    email varchar(512) NOT NULL,
    phone varchar(32) NOT NULL,
    country varchar(8) NOT NULL,
    email_code varchar(16) NOT NULL,
    verified boolean DEFAULT FALSE NOT NULL,
    email_sent_time bigint DEFAULT 0 NOT NULL,
    text_sent integer DEFAULT 0 NOT NULL,
    last_seen bigint DEFAULT 0 NOT NULL,
    last_balance_warning_time bigint DEFAULT 0 NOT NULL
);

INSERT INTO clients
    (mesh_ip, wg_pubkey, wg_port, internal_ip, nickname, email, phone, country, email_code, verified, email_sent_time, text_sent, last_seen, last_balance_warning_time)
SELECT mesh_ip, wg_pubkey, wg_port, internal_ip, nickname, email, phone, country, email_code, verified, email_sent_time, text_sent, last_seen, last_balance_warning_time
FROM clients_old;

DROP TABLE clients_old;