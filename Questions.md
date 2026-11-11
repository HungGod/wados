why is the site in the wados host folder? Currently the wadspaces-client holds the site, so it seems a bit redundant to have it in the project file.

What is the purpose of art.py? Is it still necessary to have in the project since the backgrounds have already been created? If it is the only python file in the project it should probably just be removed. 

From my brief scan it seems like c++ and rust are the only necessary dependencies and node js and python can be removed.

There is some bloat in assets like codex icon, docs-rs.png, notion.png, chatgpt.svg, calendar.png, figma.svg, 

some of the .claude files linger about the project and can probably be purged.

How does wadbrowser work? Can it run without the cactus-browser repo? 

