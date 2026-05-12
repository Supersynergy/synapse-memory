import pymysql
c = pymysql.connect(host='127.0.0.1', port=3306, user='root', password='synapse', database='wordpress')
cur = c.cursor()
cur.execute('SELECT VERSION()')
print('Synapse MySQL version:', cur.fetchone())
c.close()
