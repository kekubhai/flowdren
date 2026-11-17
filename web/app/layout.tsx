import type {Metadata} from 'next';
export const metadata:Metadata={title:'Flowdren - Let money move.',description:'Programmable treasury payments on Solana. Explore the Flowdren prototype.'};
export default function Layout({children}:{children:React.ReactNode}){return <html lang="en"><body>{children}</body></html>}