let emails = [];
let selectedEmailId = null;
const socket = new WebSocket(`ws://${window.location.host}/ws`);

// WebSocket event handlers
socket.onopen = () => {
    console.log('WebSocket connection established');
};

socket.onmessage = (event) => {
    const email = JSON.parse(event.data);
    // Check if this email already exists
    const existingIndex = emails.findIndex(e => e.id === email.id);
    if (existingIndex >= 0) {
        emails[existingIndex] = email;
    } else {
        emails.unshift(email); // Add to the beginning
    }
    renderEmailList();
};

socket.onerror = (error) => {
    console.error('WebSocket error:', error);
};

socket.onclose = () => {
    console.log('WebSocket connection closed');
};

// Fetch emails on load
fetch('/api/emails')
    .then(response => response.json())
    .then(data => {
        emails = data.sort((a, b) => new Date(b.received_at) - new Date(a.received_at));
        renderEmailList();
    })
    .catch(error => console.error('Error fetching emails:', error));

// Render email list
function renderEmailList() {
    const emailList = document.getElementById('email-list');
    emailList.innerHTML = '';

    if (emails.length === 0) {
        emailList.innerHTML = '<div class="email-item">No emails received</div>';
        return;
    }

    emails.forEach(email => {
        const emailItem = document.createElement('div');
        emailItem.className = `email-item ${email.id === selectedEmailId ? 'selected' : ''}`;
        emailItem.dataset.id = email.id;

        const fromRow = document.createElement('div');
        fromRow.className = 'email-from-row';

        const from = document.createElement('div');
        from.className = 'email-from';
        from.textContent = email.from;
        fromRow.appendChild(from);

        const date = new Date(email.received_at);
        const timeStr = date.toLocaleTimeString();
        const time = document.createElement('div');
        time.className = 'email-time';
        time.textContent = timeStr;
        fromRow.appendChild(time);

        emailItem.appendChild(fromRow);

        const subject = document.createElement('div');
        subject.className = 'email-subject';
        subject.textContent = email.subject;
        emailItem.appendChild(subject);

        const to = document.createElement('div');
        to.className = 'email-to';
        to.textContent = `To: ${email.to.join(', ')}`;
        emailItem.appendChild(to);

        emailItem.addEventListener('click', () => {
            selectEmail(email.id);
        });

        emailList.appendChild(emailItem);
    });
}

// Select an email
function selectEmail(id) {
    selectedEmailId = id;
    const email = emails.find(e => e.id === id);

    if (!email) return;

    // Update UI
    document.querySelectorAll('.email-item').forEach(item => {
        item.classList.toggle('selected', item.dataset.id === id);
    });

    document.getElementById('no-email-selected').style.display = 'none';
    document.getElementById('email-details').style.display = 'block';

    // Fill email details
    document.getElementById('email-subject').textContent = email.subject;
    document.getElementById('email-from').textContent = `From: ${email.from}`;
    document.getElementById('email-to').textContent = `To: ${email.to.join(', ')}`;

    const date = new Date(email.received_at);
    document.getElementById('email-time').textContent = `Received: ${date.toLocaleString()}`;

    // Fill content tabs
    if (email.html_body) {
        const htmlFrame = document.getElementById('html-frame');
        htmlFrame.srcdoc = email.html_body;
    } else {
        document.getElementById('html-frame').srcdoc = '<p>No HTML content</p>';
    }

    document.getElementById('text-content').textContent = email.text_body || 'No text content';

    const headersText = Object.entries(email.headers)
        .map(([key, value]) => `${key}: ${value}`)
        .join('\n');

    document.getElementById('headers-content').innerHTML = headersText;

    // Fill attachments tab
    const attachmentsItems = document.getElementById('attachments-items');
    const noAttachments = document.getElementById('no-attachments');

    attachmentsItems.innerHTML = '';

    if (email.attachments && email.attachments.length > 0) {
        noAttachments.style.display = 'none';

        email.attachments.forEach(attachment => {
            const li = document.createElement('li');
            li.className = 'attachment-item';

            const link = document.createElement('a');
            link.href = `/api/emails/${email.id}/attachments/${attachment.id}`;
            link.className = 'attachment-link';
            link.target = '_blank';

            const icon = document.createElement('span');
            icon.className = 'icon';
            icon.innerHTML = '&#128206;';
            link.appendChild(icon);

            const info = document.createElement('div');
            info.className = 'attachment-info';

            const name = document.createElement('div');
            name.className = 'attachment-name';
            name.textContent = attachment.filename;
            info.appendChild(name);

            const meta = document.createElement('div');
            meta.className = 'attachment-meta';
            meta.textContent = `${attachment.content_type}, ${formatFileSize(attachment.size)}`;
            info.appendChild(meta);

            link.appendChild(info);
            li.appendChild(link);
            attachmentsItems.appendChild(li);
        });
    } else {
        noAttachments.style.display = 'block';
    }
}

// Tab switching
document.querySelectorAll('.tab-button').forEach(button => {
    button.addEventListener('click', () => {
        // Update active tab button
        document.querySelectorAll('.tab-button').forEach(btn => {
            btn.classList.toggle('active', btn === button);
        });

        // Show active tab content
        const tabName = button.dataset.tab;
        document.querySelectorAll('.tab-content').forEach(content => {
            content.classList.toggle('active', content.id === `tab-${tabName}`);
        });
    });
});

// Format file size to human readable format
function formatFileSize(bytes) {
    if (bytes === 0) return '0 Bytes';

    const k = 1024;
    const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));

    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
}

// Clear all emails
document.getElementById('clear-all').addEventListener('click', () => {
    if (confirm('Are you sure you want to delete all emails?')) {
        fetch('/api/emails', {
            method: 'POST'
        })
        .then(() => {
            emails = [];
            selectedEmailId = null;
            renderEmailList();
            document.getElementById('no-email-selected').style.display = 'flex';
            document.getElementById('email-details').style.display = 'none';
        })
        .catch(error => console.error('Error clearing emails:', error));
    }
});
