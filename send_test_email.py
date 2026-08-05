#!/usr/bin/env python3
import smtplib
from email.mime.multipart import MIMEMultipart
from email.mime.text import MIMEText
from email.mime.base import MIMEBase
from email import encoders
import datetime

# Create message
msg = MIMEMultipart()
msg['From'] = 'files@inboxnull.example.com'
msg['To'] = '21c179d70d4a07a7@inboxnegative.com'
msg['Subject'] = 'Test Email with Attachment'

# Email body
body = 'This email contains a test attachment.'
msg.attach(MIMEText(body, 'plain'))

# Create attachment
attachment_content = f"Hello from InboxNull! This is a test attachment created at {datetime.datetime.now()}"
part = MIMEBase('text', 'plain')
part.set_payload(attachment_content.encode('utf-8'))
encoders.encode_base64(part)
part.add_header('Content-Disposition', 'attachment', filename='test_attachment.txt')
msg.attach(part)

# Send email
server = smtplib.SMTP('localhost', 2525)
server.sendmail(msg['From'], msg['To'], msg.as_string())
server.quit()

print(f"Email with attachment sent to {msg['To']}")
